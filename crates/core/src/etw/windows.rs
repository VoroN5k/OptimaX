//! FFI до ETW: системна сесія реального часу з подіями DPC/ISR і список модулів ядра.
#![allow(unsafe_code)]

use super::{Collector, DpcKind, EtwError, RawEvent};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;
use windows::core::{GUID, PCWSTR, PWSTR};
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Diagnostics::Etw::{
    CloseTrace, ControlTraceW, OpenTraceW, PerfInfoGuid, ProcessTrace, StartTraceW,
    CONTROLTRACE_HANDLE, EVENT_HEADER_FLAG_32_BIT_HEADER, EVENT_RECORD, EVENT_TRACE_CONTROL_STOP,
    EVENT_TRACE_FLAG_DPC, EVENT_TRACE_FLAG_INTERRUPT, EVENT_TRACE_LOGFILEW, EVENT_TRACE_PROPERTIES,
    EVENT_TRACE_REAL_TIME_MODE, EVENT_TRACE_SYSTEM_LOGGER_MODE, PROCESSTRACE_HANDLE,
    PROCESS_TRACE_MODE_EVENT_RECORD, PROCESS_TRACE_MODE_RAW_TIMESTAMP,
    PROCESS_TRACE_MODE_REAL_TIME, WNODE_FLAG_TRACED_GUID,
};
use windows::Win32::System::ProcessStatus::{EnumDeviceDrivers, GetDeviceDriverBaseNameW};

/// Ім'я сесії — лише наше, тож зупинити «залишену» сесію з цим ім'ям безпечно.
const SESSION_NAME: &str = "OptimaX DPC Trace";
/// GUID сесії (для системного логера — довільний, але сталий).
const SESSION_GUID: GUID = GUID::from_u128(0x5c1f_7a3e_0b6d_4e2a_9d41_7f3c_8e2b_a610);
/// `WNODE_HEADER::ClientContext`: 1 — час подій у тиках QPC.
const CLOCK_QPC: u32 = 1;
/// `INVALID_PROCESSTRACE_HANDLE` для 64-бітного процесу.
const INVALID_TRACE: u64 = u64::MAX;
/// Скільки чекати завершення потоку обробки після зупинки сесії.
const JOIN_TIMEOUT: Duration = Duration::from_secs(3);

/// Коди операцій класу `PerfInfo`: 66 ThreadDPC, 68 DPC, 69 TimerDPC, 67 ISR, 50 ISR-MSI.
fn kind_of(opcode: u8) -> Option<DpcKind> {
    match opcode {
        66 | 68 | 69 => Some(DpcKind::Dpc),
        50 | 67 => Some(DpcKind::Isr),
        _ => None,
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn check(call: &'static str, r: WIN32_ERROR) -> Result<(), EtwError> {
    if r == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(EtwError::Api { call, code: r.0 })
    }
}

/// `EVENT_TRACE_PROPERTIES` + місце для імені сесії, вирівняне на 8.
struct Properties(Vec<u64>);

impl Properties {
    fn new(name: &[u16]) -> Self {
        let head = std::mem::size_of::<EVENT_TRACE_PROPERTIES>();
        let total = head + std::mem::size_of_val(name);
        let mut buf = vec![0u64; total.div_ceil(8)];
        let p = buf.as_mut_ptr().cast::<EVENT_TRACE_PROPERTIES>();
        // SAFETY: буфер щонайменше `head` байт, вирівняний на 8 і заповнений нулями — валідне
        // значення POD-структури; пишемо поля через сирий вказівник у межах буфера.
        unsafe {
            (*p).Wnode.BufferSize = u32::try_from(buf.len() * 8).unwrap_or(u32::MAX);
            (*p).Wnode.Flags = WNODE_FLAG_TRACED_GUID;
            (*p).Wnode.ClientContext = CLOCK_QPC;
            (*p).Wnode.Guid = SESSION_GUID;
            (*p).BufferSize = 64; // КБ
            (*p).MinimumBuffers = 32;
            (*p).MaximumBuffers = 256;
            (*p).FlushTimer = 1; // с
            (*p).LogFileMode = EVENT_TRACE_REAL_TIME_MODE | EVENT_TRACE_SYSTEM_LOGGER_MODE;
            (*p).EnableFlags = EVENT_TRACE_FLAG_DPC | EVENT_TRACE_FLAG_INTERRUPT;
            (*p).LoggerNameOffset = u32::try_from(head).unwrap_or(0);
        }
        Self(buf)
    }

    fn ptr(&mut self) -> *mut EVENT_TRACE_PROPERTIES {
        self.0.as_mut_ptr().cast()
    }

    /// Втрачені події й буфери, як їх повертає `ControlTrace(STOP)`.
    fn lost(&self) -> u64 {
        let p = self.0.as_ptr().cast::<EVENT_TRACE_PROPERTIES>();
        // SAFETY: буфер містить ініціалізовану структуру (див. `new`).
        let (events, rt) = unsafe { ((*p).EventsLost, (*p).RealTimeBuffersLost) };
        u64::from(events) + u64::from(rt)
    }
}

pub(super) struct Session {
    control: CONTROLTRACE_HANDLE,
    name: Vec<u16>,
    trace: PROCESSTRACE_HANDLE,
    /// Дескриптор трасування вже закрито (або ще не відкрито) — хто перший, той і закриває.
    closed: Arc<AtomicBool>,
    done: mpsc::Receiver<Box<Collector>>,
    /// Потік обробки ще не повернув колектор.
    pending: bool,
    stopped: bool,
}

/// Вказівник на колектор для потоку обробки (`*mut` сам по собі не `Send`).
struct CollectorPtr(*mut Collector);
// SAFETY: колектором користується лише потік обробки (зворотні виклики ProcessTrace йдуть у
// його ж потоці); володіння повертається через канал після завершення ProcessTrace.
unsafe impl Send for CollectorPtr {}

impl Session {
    pub(super) fn start(collector: Collector) -> Result<Self, EtwError> {
        let name = wide(SESSION_NAME);
        let mut control = CONTROLTRACE_HANDLE::default();
        let mut props = Properties::new(&name);
        // SAFETY: `props` — коректний буфер властивостей, ім'я нуль-terminated і живе до кінця виклику.
        let mut r = unsafe { StartTraceW(&mut control, PCWSTR(name.as_ptr()), props.ptr()) };
        if r == ERROR_ALREADY_EXISTS {
            // Сесія з нашим ім'ям лишилась після аварійного завершення — зупиняємо й пробуємо знову.
            let mut stop = Properties::new(&name);
            // SAFETY: як вище; керування за іменем (дескриптор нульовий).
            let _ = unsafe {
                ControlTraceW(
                    CONTROLTRACE_HANDLE::default(),
                    PCWSTR(name.as_ptr()),
                    stop.ptr(),
                    EVENT_TRACE_CONTROL_STOP,
                )
            };
            props = Properties::new(&name);
            // SAFETY: як вище.
            r = unsafe { StartTraceW(&mut control, PCWSTR(name.as_ptr()), props.ptr()) };
        }
        check("StartTraceW", r)?;

        let mut session = Self {
            control,
            name,
            trace: PROCESSTRACE_HANDLE {
                Value: INVALID_TRACE,
            },
            closed: Arc::new(AtomicBool::new(true)),
            done: mpsc::channel().1,
            pending: false,
            stopped: false,
        };

        let raw = Box::into_raw(Box::new(collector));
        let mut logfile = EVENT_TRACE_LOGFILEW {
            LoggerName: PWSTR(session.name.as_mut_ptr()),
            Context: raw.cast(),
            ..Default::default()
        };
        logfile.Anonymous1.ProcessTraceMode = PROCESS_TRACE_MODE_REAL_TIME
            | PROCESS_TRACE_MODE_EVENT_RECORD
            | PROCESS_TRACE_MODE_RAW_TIMESTAMP;
        logfile.Anonymous2.EventRecordCallback = Some(on_event);
        // SAFETY: структура повністю ініціалізована; ім'я сесії живе в `session` до Drop.
        let trace = unsafe { OpenTraceW(&mut logfile) };
        if trace.Value == INVALID_TRACE {
            // SAFETY: `raw` щойно отримано з Box::into_raw і більше ніде не використовується.
            drop(unsafe { Box::from_raw(raw) });
            // SAFETY: без аргументів; читає код помилки цього потоку.
            let code = unsafe { windows::Win32::Foundation::GetLastError() }.0;
            return Err(EtwError::Api {
                call: "OpenTraceW",
                code,
            });
        }
        session.trace = trace;
        session.closed.store(false, Ordering::Release);

        let (tx, rx) = mpsc::channel();
        session.done = rx;
        let closed = session.closed.clone();
        let ptr = CollectorPtr(raw);
        let spawned = std::thread::Builder::new()
            .name("optimax-etw-dpc".into())
            .spawn(move || {
                let ptr = ptr;
                // SAFETY: дескриптор відкрито OpenTraceW; колектор живий (Box), доступ до нього
                // лише з цього потоку через зворотний виклик.
                let r = unsafe { ProcessTrace(&[trace], None, None) };
                if r != ERROR_SUCCESS {
                    tracing::debug!("ProcessTrace: {}", r.0);
                }
                if !closed.swap(true, Ordering::AcqRel) {
                    // SAFETY: дескриптор ще не закрито (прапорець).
                    let _ = unsafe { CloseTrace(trace) };
                }
                // SAFETY: ProcessTrace завершився — зворотних викликів більше не буде;
                // вказівник отримано з Box::into_raw.
                let collector = unsafe { Box::from_raw(ptr.0) };
                let _ = tx.send(collector);
            });
        if let Err(e) = spawned {
            tracing::warn!("потік ETW не запустився: {e}");
            // Колектор свідомо не звільняємо: дескриптор трасування відкрито, і після CloseTrace
            // формально ще можливий зворотний виклик. Витік одноразовий і малий.
            session.close_trace();
            return Err(EtwError::Api {
                call: "spawn",
                code: 0,
            });
        }
        session.pending = true;
        Ok(session)
    }

    fn close_trace(&self) {
        if !self.closed.swap(true, Ordering::AcqRel) {
            // SAFETY: дескриптор відкрито й ще не закрито (прапорець).
            let _ = unsafe { CloseTrace(self.trace) };
        }
    }

    /// Зупиняє сесію; повертає колектор і кількість втрачених подій.
    fn stop_session(&mut self) -> u64 {
        if self.stopped {
            return 0;
        }
        self.stopped = true;
        let mut props = Properties::new(&self.name);
        // SAFETY: дескриптор сесії отримано від StartTraceW; буфер властивостей коректний.
        let r = unsafe {
            ControlTraceW(
                self.control,
                PCWSTR::null(),
                props.ptr(),
                EVENT_TRACE_CONTROL_STOP,
            )
        };
        if r != ERROR_SUCCESS {
            tracing::warn!("не вдалося зупинити ETW-сесію DPC: {}", r.0);
        }
        props.lost()
    }

    pub(super) fn stop(mut self) -> Option<(Collector, u64)> {
        let lost = self.stop_session();
        self.join().map(|c| (*c, lost))
    }

    /// Чекає, поки потік обробки поверне колектор.
    fn join(&mut self) -> Option<Box<Collector>> {
        if !self.pending {
            return None;
        }
        let collector = match self.done.recv_timeout(JOIN_TIMEOUT) {
            Ok(c) => Some(c),
            Err(_) => {
                // ProcessTrace не повернувся сам — закриваємо дескриптор, це його розблоковує.
                self.close_trace();
                self.done.recv_timeout(JOIN_TIMEOUT).ok()
            }
        };
        self.pending = false;
        collector
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop_session();
        drop(self.join());
        self.close_trace();
    }
}

/// Зворотний виклик ProcessTrace для кожної події.
unsafe extern "system" fn on_event(record: *mut EVENT_RECORD) {
    // SAFETY: ETW передає валідний вказівник на запис на час виклику.
    let Some(rec) = (unsafe { record.as_ref() }) else {
        return;
    };
    let h = &rec.EventHeader;
    if h.ProviderId != PerfInfoGuid {
        return;
    }
    let Some(kind) = kind_of(h.EventDescriptor.Opcode) else {
        return;
    };
    let ptr_size = if u32::from(h.Flags) & EVENT_HEADER_FLAG_32_BIT_HEADER != 0 {
        4
    } else {
        8
    };
    let need = 8 + ptr_size;
    if rec.UserData.is_null() || usize::from(rec.UserDataLength) < need || rec.UserContext.is_null()
    {
        return;
    }
    // SAFETY: UserData містить щонайменше `need` байт (перевірено UserDataLength).
    let data = unsafe { std::slice::from_raw_parts(rec.UserData.cast::<u8>(), need) };
    let start = i64::from_le_bytes(data[..8].try_into().unwrap_or_default());
    let routine = if ptr_size == 8 {
        u64::from_le_bytes(data[8..16].try_into().unwrap_or_default())
    } else {
        u64::from(u32::from_le_bytes(
            data[8..12].try_into().unwrap_or_default(),
        ))
    };
    // SAFETY: UserContext — наш `*mut Collector` з OpenTraceW; живий, поки працює ProcessTrace,
    // і використовується лише з цього потоку.
    let collector = unsafe { &mut *rec.UserContext.cast::<Collector>() };
    collector.push(RawEvent {
        start_qpc: start,
        end_qpc: h.TimeStamp,
        routine,
        kind,
    });
}

/// Модулі ядра: (база, ім'я файлу). Порожньо, якщо список недоступний.
pub(super) fn kernel_modules() -> Vec<(u64, String)> {
    let mut bases: Vec<*mut core::ffi::c_void> = vec![std::ptr::null_mut(); 1024];
    for _ in 0..3 {
        let mut needed = 0u32;
        let cb = u32::try_from(std::mem::size_of_val(bases.as_slice())).unwrap_or(u32::MAX);
        // SAFETY: буфер на `cb` байт, вихідний параметр локальний.
        if unsafe { EnumDeviceDrivers(bases.as_mut_ptr(), cb, &mut needed) }.is_err() {
            return Vec::new();
        }
        let n = needed as usize / std::mem::size_of::<*mut core::ffi::c_void>();
        if n <= bases.len() {
            bases.truncate(n);
            break;
        }
        bases = vec![std::ptr::null_mut(); n + 64];
    }
    let mut out = Vec::with_capacity(bases.len());
    let mut buf = [0u16; 260];
    for base in bases {
        if base.is_null() {
            continue;
        }
        // SAFETY: база з EnumDeviceDrivers, буфер локальний.
        let len = unsafe { GetDeviceDriverBaseNameW(base, &mut buf) } as usize;
        if len > 0 {
            let name = String::from_utf16_lossy(&buf[..len.min(buf.len())]);
            out.push((base as u64, name));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::DpcTracer;
    use crate::clock::qpc_frequency;

    /// Потребує прав адміністратора: `cargo test -p optimax-core etw -- --ignored --nocapture`
    /// у PowerShell, запущеному від імені адміністратора.
    #[test]
    #[ignore = "потрібні права адміністратора й реальний Windows"]
    fn dpc_trace_on_real_system() {
        let modules = super::kernel_modules();
        println!("модулів ядра: {}", modules.len());
        let tracer = DpcTracer::start(qpc_frequency()).expect("старт сесії");
        std::thread::sleep(std::time::Duration::from_secs(5));
        let t = tracer.stop().expect("підсумок");
        println!(
            "подій: {} ({}/с), втрачено: {}, довгих (≥100 мкс): {}",
            t.events,
            t.events / 5,
            t.lost,
            t.long.len()
        );
        for d in t.drivers.iter().take(10) {
            println!(
                "{:<24} DPC {:>7} ISR {:>7} сума {:>9.0} мкс макс {:>7.1} мкс",
                d.name.as_deref().unwrap_or("?"),
                d.dpc_count,
                d.isr_count,
                d.total_us,
                d.max_us
            );
        }
        assert!(t.events > 0, "події DPC не надходять");
        assert!(
            t.drivers.iter().any(|d| d.name.is_some()),
            "драйвери не визначено"
        );
    }
}
