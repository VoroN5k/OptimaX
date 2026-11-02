//! FFI до ETW: сесії реального часу (системна з подіями ядра або звичайна з провайдерами),
//! пошук провайдера за назвою і назви подій (TDH).
#![allow(unsafe_code)]

use crate::{EventKey, SessionKind, TraceError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;
use windows::core::{GUID, PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, WIN32_ERROR,
};
use windows::Win32::System::Diagnostics::Etw::{
    CloseTrace, ControlTraceW, EnableTraceEx2, OpenTraceW, ProcessTrace, StartTraceW,
    TdhEnumerateProviders, TdhGetEventInformation, CONTROLTRACE_HANDLE,
    EVENT_CONTROL_CODE_ENABLE_PROVIDER, EVENT_HEADER_FLAG_32_BIT_HEADER, EVENT_RECORD,
    EVENT_TRACE_CONTROL_QUERY, EVENT_TRACE_CONTROL_STOP, EVENT_TRACE_FLAG, EVENT_TRACE_LOGFILEW,
    EVENT_TRACE_PROPERTIES, EVENT_TRACE_REAL_TIME_MODE, EVENT_TRACE_SYSTEM_LOGGER_MODE,
    PROCESSTRACE_HANDLE, PROCESS_TRACE_MODE_EVENT_RECORD, PROCESS_TRACE_MODE_RAW_TIMESTAMP,
    PROCESS_TRACE_MODE_REAL_TIME, PROVIDER_ENUMERATION_INFO, TRACE_EVENT_INFO, TRACE_PROVIDER_INFO,
    WNODE_FLAG_TRACED_GUID,
};

/// `WNODE_HEADER::ClientContext`: 1 — час подій у тиках QPC (як у PresentMon).
const CLOCK_QPC: u32 = 1;
/// GUID системної сесії (для системного логера — довільний, але сталий; інший, ніж у DPC).
const KERNEL_SESSION_GUID: GUID = GUID::from_u128(0x7b2e_41c9_5d0a_4f86_b3e1_2c9d_6a04_f871);
/// `INVALID_PROCESSTRACE_HANDLE` для 64-бітного процесу.
const INVALID_TRACE: u64 = u64::MAX;
/// Скільки чекати завершення потоку обробки після зупинки сесії.
const JOIN_TIMEOUT: Duration = Duration::from_secs(3);

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn check(call: &'static str, r: WIN32_ERROR) -> Result<(), TraceError> {
    if r == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(TraceError::Api { call, code: r.0 })
    }
}

/// Подія ETW на час зворотного виклику.
pub struct Record<'a> {
    rec: &'a EVENT_RECORD,
}

impl Record<'_> {
    /// Вид події.
    pub fn key(&self) -> EventKey {
        let h = &self.rec.EventHeader;
        EventKey {
            provider: h.ProviderId.to_u128(),
            id: h.EventDescriptor.Id,
            opcode: h.EventDescriptor.Opcode,
            version: h.EventDescriptor.Version,
        }
    }

    /// Маска ключових слів події (для вибору ключових слів провайдера).
    pub fn keyword(&self) -> u64 {
        self.rec.EventHeader.EventDescriptor.Keyword
    }

    /// Процес із заголовка (для подій ядра не завжди означає «чия подія»).
    pub fn pid(&self) -> u32 {
        self.rec.EventHeader.ProcessId
    }

    /// Потік із заголовка.
    pub fn tid(&self) -> u32 {
        self.rec.EventHeader.ThreadId
    }

    /// Час, тики QPC.
    pub fn timestamp(&self) -> i64 {
        self.rec.EventHeader.TimeStamp
    }

    /// Логічний процесор, на якому записано подію.
    pub fn cpu(&self) -> u8 {
        // SAFETY: обидва варіанти об'єднання — цілі числа; молодший байт — номер процесора.
        unsafe { self.rec.BufferContext.Anonymous.Anonymous.ProcessorNumber }
    }

    /// Розмір вказівника в даних події (4 або 8).
    pub fn pointer_size(&self) -> usize {
        if u32::from(self.rec.EventHeader.Flags) & EVENT_HEADER_FLAG_32_BIT_HEADER != 0 {
            4
        } else {
            8
        }
    }

    /// Дані події.
    pub fn data(&self) -> &[u8] {
        let len = usize::from(self.rec.UserDataLength);
        if self.rec.UserData.is_null() || len == 0 {
            return &[];
        }
        // SAFETY: ETW гарантує `UserDataLength` байт за `UserData` на час зворотного виклику.
        unsafe { std::slice::from_raw_parts(self.rec.UserData.cast::<u8>(), len) }
    }

    /// Назви події з опису Windows (TDH): завдання, операція, подія.
    pub fn names(&self) -> Option<(String, String, String)> {
        event_names(self.rec)
    }
}

/// Рядок UTF-16 з нулем у кінці за зміщенням `offset` у буфері `buf` (байт).
fn wide_at(buf: &[u8], offset: u32) -> String {
    let start = offset as usize;
    if offset == 0 || start >= buf.len() {
        return String::new();
    }
    let units: Vec<u16> = buf[start..]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16_lossy(&units).trim().to_string()
}

/// Назви події (завдання, операція, подія) з TDH. `None` — опису немає.
pub fn event_names(rec: &EVENT_RECORD) -> Option<(String, String, String)> {
    let mut size = 0u32;
    // SAFETY: запит розміру: буфер не передаємо, `size` — локальний.
    let r = unsafe { TdhGetEventInformation(rec, None, None, &mut size) };
    if r != ERROR_INSUFFICIENT_BUFFER.0 || size == 0 {
        return None;
    }
    let mut buf = vec![0u64; (size as usize).div_ceil(8)];
    let info = buf.as_mut_ptr().cast::<TRACE_EVENT_INFO>();
    // SAFETY: буфер щонайменше `size` байт, вирівняний на 8.
    let r = unsafe { TdhGetEventInformation(rec, None, Some(info), &mut size) };
    if r != ERROR_SUCCESS.0 {
        return None;
    }
    // SAFETY: TDH заповнив структуру на початку буфера.
    let (task, opcode, event) = unsafe {
        (
            (*info).TaskNameOffset,
            (*info).OpcodeNameOffset,
            (*info).Anonymous1.EventNameOffset,
        )
    };
    // SAFETY: читаємо власний буфер як байти в його межах.
    let bytes = unsafe {
        std::slice::from_raw_parts(
            buf.as_ptr().cast::<u8>(),
            (size as usize).min(buf.len() * 8),
        )
    };
    Some((
        wide_at(bytes, task),
        wide_at(bytes, opcode),
        wide_at(bytes, event),
    ))
}

/// GUID провайдера за назвою (`Microsoft-Windows-DxgKrnl`), з переліку зареєстрованих у системі.
pub fn provider_guid(name: &str) -> Option<u128> {
    let mut size = 0u32;
    // SAFETY: запит розміру.
    let r = unsafe { TdhEnumerateProviders(None, &mut size) };
    if r != ERROR_INSUFFICIENT_BUFFER.0 || size == 0 {
        return None;
    }
    let mut buf = vec![0u64; (size as usize).div_ceil(8) + 1];
    let info = buf.as_mut_ptr().cast::<PROVIDER_ENUMERATION_INFO>();
    // SAFETY: буфер щонайменше `size` байт, вирівняний на 8.
    let r = unsafe { TdhEnumerateProviders(Some(info), &mut size) };
    if r != ERROR_SUCCESS.0 {
        return None;
    }
    let total = (size as usize).min(buf.len() * 8);
    // SAFETY: читаємо власний буфер як байти в його межах.
    let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), total) };
    // SAFETY: TDH заповнив заголовок переліку.
    let count = unsafe { (*info).NumberOfProviders } as usize;
    let first = std::mem::offset_of!(PROVIDER_ENUMERATION_INFO, TraceProviderInfoArray);
    let item = std::mem::size_of::<TRACE_PROVIDER_INFO>();
    (0..count).find_map(|i| {
        let at = first + i * item;
        if at + item > total {
            return None;
        }
        // SAFETY: елемент у межах буфера (перевірено); читаємо без вимог до вирівнювання.
        let p = unsafe {
            std::ptr::read_unaligned(bytes.as_ptr().add(at).cast::<TRACE_PROVIDER_INFO>())
        };
        wide_at(bytes, p.ProviderNameOffset)
            .eq_ignore_ascii_case(name)
            .then(|| p.ProviderGuid.to_u128())
    })
}

/// `EVENT_TRACE_PROPERTIES` + місце для імені сесії, вирівняне на 8.
struct Properties(Vec<u64>);

impl Properties {
    fn new(name: &[u16], kind: Option<&SessionKind>) -> Self {
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
            (*p).LoggerNameOffset = u32::try_from(head).unwrap_or(0);
            match kind {
                Some(SessionKind::Kernel { flags }) => {
                    // Перемикань потоків — десятки тисяч за секунду: більші буфери.
                    (*p).BufferSize = 256; // КБ
                    (*p).MinimumBuffers = 64;
                    (*p).MaximumBuffers = 512;
                    (*p).LogFileMode = EVENT_TRACE_REAL_TIME_MODE | EVENT_TRACE_SYSTEM_LOGGER_MODE;
                    (*p).EnableFlags = EVENT_TRACE_FLAG(*flags);
                    (*p).Wnode.Guid = KERNEL_SESSION_GUID;
                }
                Some(SessionKind::Providers(_)) => {
                    (*p).BufferSize = 128;
                    (*p).MinimumBuffers = 32;
                    (*p).MaximumBuffers = 256;
                    (*p).LogFileMode = EVENT_TRACE_REAL_TIME_MODE;
                }
                None => {}
            }
            (*p).FlushTimer = 1; // с
        }
        Self(buf)
    }

    fn ptr(&mut self) -> *mut EVENT_TRACE_PROPERTIES {
        self.0.as_mut_ptr().cast()
    }

    /// Втрачені події й буфери.
    fn lost(&self) -> u64 {
        let p = self.0.as_ptr().cast::<EVENT_TRACE_PROPERTIES>();
        // SAFETY: буфер містить ініціалізовану структуру (див. `new`).
        let (events, rt) = unsafe { ((*p).EventsLost, (*p).RealTimeBuffersLost) };
        u64::from(events) + u64::from(rt)
    }
}

/// Обробник подій (викликається з потоку ProcessTrace).
type Sink = dyn Fn(&Record<'_>) + Send + Sync;

/// Вказівник на обробник для потоку обробки.
struct SinkPtr(*const Box<Sink>);
// SAFETY: вказівник отримано з `Arc::into_raw`; обробник `Send + Sync`. Посилання звільняється в
// тому ж потоці після ProcessTrace.
unsafe impl Send for SinkPtr {}

/// Запущена сесія трасування. Зупиняється [`Session::stop`] або при знищенні.
pub struct Session {
    control: CONTROLTRACE_HANDLE,
    name: Vec<u16>,
    trace: PROCESSTRACE_HANDLE,
    closed: Arc<AtomicBool>,
    done: mpsc::Receiver<()>,
    pending: bool,
    stopped: bool,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Session")
    }
}

impl Session {
    /// Запускає сесію з іменем `name` (лише наше: «залишену» сесію з ним буде зупинено).
    /// `sink` отримує кожну подію в потоці обробки — має бути швидким.
    pub fn start(
        name: &str,
        kind: &SessionKind,
        sink: impl Fn(&Record<'_>) + Send + Sync + 'static,
    ) -> Result<Self, TraceError> {
        let wname = wide(name);
        let mut control = CONTROLTRACE_HANDLE::default();
        let mut props = Properties::new(&wname, Some(kind));
        // SAFETY: `props` — коректний буфер властивостей, ім'я нуль-terminated.
        let mut r = unsafe { StartTraceW(&mut control, PCWSTR(wname.as_ptr()), props.ptr()) };
        if r == ERROR_ALREADY_EXISTS {
            let mut stop = Properties::new(&wname, None);
            // SAFETY: керування за іменем (дескриптор нульовий), буфер коректний.
            let _ = unsafe {
                ControlTraceW(
                    CONTROLTRACE_HANDLE::default(),
                    PCWSTR(wname.as_ptr()),
                    stop.ptr(),
                    EVENT_TRACE_CONTROL_STOP,
                )
            };
            props = Properties::new(&wname, Some(kind));
            // SAFETY: як вище.
            r = unsafe { StartTraceW(&mut control, PCWSTR(wname.as_ptr()), props.ptr()) };
        }
        check("StartTraceW", r)?;

        let mut session = Self {
            control,
            name: wname,
            trace: PROCESSTRACE_HANDLE {
                Value: INVALID_TRACE,
            },
            closed: Arc::new(AtomicBool::new(true)),
            done: mpsc::channel().1,
            pending: false,
            stopped: false,
        };

        if let SessionKind::Providers(providers) = kind {
            for p in providers {
                let guid = GUID::from_u128(p.guid);
                // SAFETY: дескриптор сесії щойно отримано від StartTraceW; GUID локальний.
                let r = unsafe {
                    EnableTraceEx2(
                        session.control,
                        &guid,
                        EVENT_CONTROL_CODE_ENABLE_PROVIDER.0,
                        p.level,
                        p.keywords,
                        0,
                        0,
                        None,
                    )
                };
                check("EnableTraceEx2", r)?;
            }
        }

        let sink: Arc<Box<Sink>> = Arc::new(Box::new(sink));
        let raw = Arc::into_raw(sink);
        let mut logfile = EVENT_TRACE_LOGFILEW {
            LoggerName: PWSTR(session.name.as_mut_ptr()),
            Context: raw.cast_mut().cast(),
            ..Default::default()
        };
        logfile.Anonymous1.ProcessTraceMode = PROCESS_TRACE_MODE_REAL_TIME
            | PROCESS_TRACE_MODE_EVENT_RECORD
            | PROCESS_TRACE_MODE_RAW_TIMESTAMP;
        logfile.Anonymous2.EventRecordCallback = Some(on_event);
        // SAFETY: структура повністю ініціалізована; ім'я сесії живе в `session` до Drop.
        let trace = unsafe { OpenTraceW(&mut logfile) };
        if trace.Value == INVALID_TRACE {
            // SAFETY: `raw` щойно отримано з Arc::into_raw; трасування не відкрито.
            drop(unsafe { Arc::from_raw(raw) });
            // SAFETY: без аргументів; код помилки цього потоку.
            let code = unsafe { windows::Win32::Foundation::GetLastError() }.0;
            return Err(TraceError::Api {
                call: "OpenTraceW",
                code,
            });
        }
        session.trace = trace;
        session.closed.store(false, Ordering::Release);

        let (tx, rx) = mpsc::channel();
        session.done = rx;
        let closed = session.closed.clone();
        let ptr = SinkPtr(raw);
        let spawned = std::thread::Builder::new()
            .name("optimax-trace".into())
            .spawn(move || {
                let ptr = ptr;
                // SAFETY: дескриптор відкрито OpenTraceW; обробник живий (посилання `ptr`).
                let r = unsafe { ProcessTrace(&[trace], None, None) };
                if r != ERROR_SUCCESS {
                    tracing::debug!("ProcessTrace: {}", r.0);
                }
                if !closed.swap(true, Ordering::AcqRel) {
                    // SAFETY: дескриптор ще не закрито (прапорець).
                    let _ = unsafe { CloseTrace(trace) };
                }
                // SAFETY: ProcessTrace завершився — зворотних викликів більше не буде;
                // вказівник з Arc::into_raw звільняється рівно один раз.
                drop(unsafe { Arc::from_raw(ptr.0) });
                let _ = tx.send(());
            });
        if spawned.is_err() {
            // Посилання на обробник свідомо не звільняємо (див. DPC-трасування в core).
            session.close_trace();
            return Err(TraceError::Api {
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

    fn control(
        &self,
        code: windows::Win32::System::Diagnostics::Etw::EVENT_TRACE_CONTROL,
    ) -> Option<u64> {
        let mut props = Properties::new(&self.name, None);
        // SAFETY: дескриптор сесії отримано від StartTraceW; буфер властивостей коректний.
        let r = unsafe { ControlTraceW(self.control, PCWSTR::null(), props.ptr(), code) };
        (r == ERROR_SUCCESS).then(|| props.lost())
    }

    /// Втрачені досі події (0, якщо запит не вдався).
    pub fn lost_so_far(&self) -> u64 {
        self.control(EVENT_TRACE_CONTROL_QUERY).unwrap_or(0)
    }

    /// Зупиняє сесію й чекає потік обробки. Повертає кількість втрачених подій.
    pub fn stop(mut self) -> u64 {
        self.finish()
    }

    fn finish(&mut self) -> u64 {
        let mut lost = 0;
        if !self.stopped {
            self.stopped = true;
            match self.control(EVENT_TRACE_CONTROL_STOP) {
                Some(l) => lost = l,
                None => tracing::warn!("не вдалося зупинити сесію трасування"),
            }
        }
        if self.pending {
            self.pending = false;
            if self.done.recv_timeout(JOIN_TIMEOUT).is_err() {
                // ProcessTrace не повернувся сам — закриваємо дескриптор, це його розблоковує.
                self.close_trace();
                let _ = self.done.recv_timeout(JOIN_TIMEOUT);
            }
        }
        self.close_trace();
        lost
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.finish();
    }
}

/// Зворотний виклик ProcessTrace для кожної події.
unsafe extern "system" fn on_event(record: *mut EVENT_RECORD) {
    // SAFETY: ETW передає валідний вказівник на запис на час виклику.
    let Some(rec) = (unsafe { record.as_ref() }) else {
        return;
    };
    if rec.UserContext.is_null() {
        return;
    }
    // SAFETY: UserContext — наш `*const Box<Sink>` з OpenTraceW (Arc::into_raw); посилання живе,
    // поки працює ProcessTrace.
    let sink = unsafe { &*rec.UserContext.cast::<Box<Sink>>() };
    sink(&Record { rec });
}
