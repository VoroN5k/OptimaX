//! Сама служба: диспетчер служб Windows, цикл каналу, перевірка клієнта, відповіді.
//!
//! Журнал служби — `%ProgramData%\OptimaX\service\service.log` (лише SYSTEM і Адміністратори).

use crate::pipe::{process_image, PipeStream};
use crate::protocol::{
    compatible, read_frame, write_frame, Command, Hello, Pong, Reply, Request, Response, PIPE_NAME,
    SERVICE_NAME,
};
use crate::{pipe_sddl, same_file, user_from_args};
use std::ffi::OsString;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::{define_windows_service, service_dispatcher};

/// Межа файлу журналу служби; на диску — щонайбільше два такі файли (`crate::logfile`).
const LOG_MAX: u64 = 2 * 1024 * 1024;

define_windows_service!(ffi_service_main, service_main);

/// Хто під'єднався: процес і його користувач (з токена процесу, не зі слів клієнта).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    /// PID клієнта.
    pub pid: u32,
    /// SID користувача клієнта (`S-1-5-21-…`).
    pub sid: String,
}

/// Виконавець команд, крім `ping` (його служба відповідає сама). Підставляє `app`, щоб цей крейт
/// не залежав від доменних крейтів.
pub type Handler = Arc<dyn Fn(&Peer, &Command) -> Reply + Send + Sync>;

/// Виконавець для `service_main`: диспетчер служб викликає звичайну функцію без стану, тож
/// виконавець передається через статичну комірку. Записується один раз до старту диспетчера,
/// далі лише читається.
static HANDLER: OnceLock<Handler> = OnceLock::new();

/// Точка входу `optimax.exe --service`: передає керування диспетчеру служб Windows.
/// Поза диспетчером (запуск вручну) повертає помилку.
pub fn run(handler: Handler) -> Result<(), crate::ServiceError> {
    let _ = HANDLER.set(handler);
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
        .map_err(|e| crate::ServiceError::Os(format!("диспетчер служб: {e}")))
}

fn init_log() {
    let Ok(dir) = optimax_core::datadir::shared_dir("service") else {
        return;
    };
    let Ok(file) = crate::logfile::CappedLog::open(&dir.join("service.log"), LOG_MAX) else {
        return;
    };
    let _ = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(std::sync::Mutex::new(file))
        .try_init();
}

fn service_main(_args: Vec<OsString>) {
    init_log();
    if let Err(e) = serve() {
        tracing::error!("служба зупинилась з помилкою: {e}");
    }
}

fn serve() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let sid = user_from_args(&args).ok_or("немає --user <SID> у конфігурації служби")?;
    let sddl = pipe_sddl(&sid).ok_or("некоректний SID")?;
    let own_exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let stop = Arc::new(AtomicBool::new(false));

    let handler_stop = stop.clone();
    let status = service_control_handler::register(SERVICE_NAME, move |c| match c {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            handler_stop.store(true, Ordering::SeqCst);
            // Розбудити потік, що чекає клієнта.
            let _ = PipeStream::connect(PIPE_NAME);
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    })
    .map_err(|e| e.to_string())?;
    let set = |state, accept| {
        let _ = status.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: accept,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::from_secs(5),
            process_id: None,
        });
    };

    let started = Instant::now();
    let handler = HANDLER.get().cloned().ok_or("немає виконавця команд")?;
    let result = listen(PIPE_NAME, &sddl, &own_exe, &stop, started, &handler, || {
        set(
            ServiceState::Running,
            ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        );
        tracing::info!(
            "служба {} запущена, канал {PIPE_NAME}",
            env!("CARGO_PKG_VERSION")
        );
    });
    set(ServiceState::Stopped, ServiceControlAccept::empty());
    tracing::info!("служба зупинена");
    result
}

fn listen(
    name: &str,
    sddl: &str,
    own_exe: &Path,
    stop: &AtomicBool,
    started: Instant,
    handler: &Handler,
    on_ready: impl FnOnce(),
) -> Result<(), String> {
    // Перший екземпляр: якщо канал уже хтось створив — не працюємо з чужим.
    let mut next = PipeStream::listen(name, sddl, true)
        .map_err(|e| format!("канал не створено (зайнятий?): {e}"))?;
    on_ready();
    while !stop.load(Ordering::SeqCst) {
        if let Err(e) = next.accept() {
            tracing::warn!("канал: {e}");
            // Клієнт міг під'єднатися й одразу закритися (ERROR_NO_DATA): такий екземпляр більше
            // нікого не прийме, а нові клієнти отримували б ERROR_PIPE_BUSY. Замінюємо його новим —
            // спершу новий, щоб ім'я каналу ні на мить не звільнялося для чужого процесу.
            let fresh = PipeStream::listen(name, sddl, false).map_err(|e| e.to_string())?;
            drop(std::mem::replace(&mut next, fresh));
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let conn = std::mem::replace(
            &mut next,
            PipeStream::listen(name, sddl, false).map_err(|e| e.to_string())?,
        );
        let exe = own_exe.to_path_buf();
        let handler = handler.clone();
        let spawned = std::thread::Builder::new()
            .name("optimax-svc-client".into())
            .spawn(move || handle(conn, &exe, started, &handler));
        if let Err(e) = spawned {
            tracing::warn!("потік клієнта не запущено: {e}");
        }
    }
    Ok(())
}

/// Обслуговує одного клієнта, поки він не закриє канал.
fn handle(mut conn: PipeStream, own_exe: &Path, started: Instant, handler: &Handler) {
    let client = conn
        .client_pid()
        .and_then(|pid| process_image(pid).map(|p| (pid, p)));
    let (pid, image) = match client {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("клієнта не визначено: {e}");
            return;
        }
    };
    if !same_file(&image, own_exe) {
        tracing::warn!("відхилено клієнта {pid}: {}", image.display());
        return;
    }
    // Чий це процес — з його токена: від цього залежать тека журналу й гілка HKCU для змін.
    let peer = match optimax_core::datadir::process_user_sid(pid) {
        Ok(sid) => Peer { pid, sid },
        Err(e) => {
            tracing::warn!("клієнт {pid}: користувача не визначено: {e}");
            return;
        }
    };
    let theirs: Hello = match read_frame(&mut conn) {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!("клієнт {pid}: немає привітання: {e}");
            return;
        }
    };
    if write_frame(&mut conn, &Hello::ours()).is_err() || !compatible(&theirs) {
        tracing::info!(
            "клієнт {pid}: версія протоколу {} не підходить",
            theirs.protocol
        );
        return;
    }
    tracing::debug!("клієнт {pid} ({}) під'єднався", theirs.version);
    loop {
        let req: Request = match read_frame(&mut conn) {
            Ok(r) => r,
            // Клієнт закрив канал або прислав незрозуміле — розрив.
            Err(_) => return,
        };
        let reply = match &req.command {
            Command::Ping => Reply::Pong(Pong {
                version: env!("CARGO_PKG_VERSION").to_string(),
                uptime_s: started.elapsed().as_secs(),
            }),
            other => handler(&peer, other),
        };
        if write_frame(&mut conn, &Response { id: req.id, reply }).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Справжній канал Windows у межах одного процесу: дескриптор безпеки з SID користувача,
    /// перевірка клієнта (той самий exe), привітання, `ping`, зупинка.
    #[test]
    fn real_pipe_handshake_and_ping() {
        let sid = optimax_core::datadir::current_user_sid().expect("SID");
        // Тест іде без прав адміністратора, а наступні екземпляри каналу створює той, хто має
        // FILE_CREATE_PIPE_INSTANCE: служба (SYSTEM) має, користувач за `pipe_sddl` — свідомо ні.
        // Тож тут користувачу повний доступ; сам `pipe_sddl` перевіряє окремий тест.
        assert!(crate::pipe_sddl(&sid).is_some());
        let sddl = format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;{sid})");
        let name = format!(r"\\.\pipe\OptimaX-test-{}", std::process::id());
        let exe = std::env::current_exe().expect("exe");
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let server = {
            let (name, stop) = (name.clone(), stop.clone());
            std::thread::spawn(move || {
                let handler: Handler = Arc::new(|peer: &Peer, c: &Command| match c {
                    Command::CpuTemp => Reply::Json(serde_json::json!(42.5)),
                    Command::JournalEntries => Reply::Json(serde_json::json!(peer.sid)),
                    _ => Reply::Error("не в тесті".into()),
                });
                listen(&name, &sddl, &exe, &stop, Instant::now(), &handler, || {
                    let _ = ready_tx.send(());
                })
            })
        };
        ready_rx.recv().expect("сервер готовий");

        let mut c = PipeStream::connect(&name).expect("з'єднання");
        assert_eq!(c.server_pid().expect("pid"), std::process::id());
        write_frame(&mut c, &Hello::ours()).expect("hello");
        let theirs: Hello = read_frame(&mut c).expect("hello служби");
        assert!(compatible(&theirs));
        write_frame(
            &mut c,
            &Request {
                id: 5,
                command: Command::Ping,
            },
        )
        .expect("ping");
        let r: Response = read_frame(&mut c).expect("pong");
        assert_eq!(r.id, 5);
        assert!(matches!(r.reply, Reply::Pong(ref p) if p.version == env!("CARGO_PKG_VERSION")));
        // Решту команд виконує підставлений виконавець.
        write_frame(
            &mut c,
            &Request {
                id: 6,
                command: Command::CpuTemp,
            },
        )
        .expect("cpu_temp");
        let r: Response = read_frame(&mut c).expect("відповідь");
        assert_eq!(r.reply, Reply::Json(serde_json::json!(42.5)));
        // Виконавець знає, чий процес під'єднався (з токена).
        write_frame(
            &mut c,
            &Request {
                id: 7,
                command: Command::JournalEntries,
            },
        )
        .expect("journal");
        let r: Response = read_frame(&mut c).expect("відповідь");
        assert_eq!(r.reply, Reply::Json(serde_json::json!(sid)));
        // Канал створив цей процес: власник — SYSTEM/Адміністратори лише з правами адміністратора.
        assert_eq!(
            c.owner_is_privileged().expect("власник"),
            optimax_core::process::is_elevated()
        );
        drop(c);

        // Клієнти, що під'єднуються й одразу закриваються, не ламають прийом наступних.
        for _ in 0..20 {
            let _ = PipeStream::connect(&name);
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut c = loop {
            match PipeStream::connect(&name) {
                Ok(c) => break c,
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => panic!("канал більше не приймає з'єднань: {e}"),
            }
        };
        write_frame(&mut c, &Hello::ours()).expect("hello");
        let theirs: Hello = read_frame(&mut c).expect("hello служби");
        assert!(compatible(&theirs));
        drop(c);

        // Другий сервер з тим самим іменем як «перший екземпляр» не створиться.
        let again = format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;{sid})");
        assert!(PipeStream::listen(&name, &again, true).is_err());

        stop.store(true, Ordering::SeqCst);
        let _ = PipeStream::connect(&name);
        assert!(server.join().expect("потік").is_ok());
    }
}
