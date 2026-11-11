//! Протокол каналу між інтерфейсом і службою (чисті типи й кадрування, без системних викликів).
//!
//! Кадр: 4 байти довжини (little-endian) + JSON. Першим повідомленням кожна сторона надсилає
//! [`Hello`]; далі клієнт шле [`Request`], служба відповідає [`Response`] з тим самим `id`.
//! Перелік команд закритий ([`Command`]): невідома команда не розбереться й з'єднання закриється.

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::io::{Read, Write};

/// Ім'я служби Windows.
pub const SERVICE_NAME: &str = "OptimaX";
/// Показуване ім'я служби.
pub const SERVICE_DISPLAY: &str = "OptimaX";
/// Ім'я каналу.
pub const PIPE_NAME: &str = r"\\.\pipe\OptimaX";
/// Версія протоколу: змінюється, коли старий клієнт і нова служба вже не зрозуміють одне одного.
pub const PROTOCOL: u32 = 2;
/// Найбільший кадр, байт: більший — помилка й розрив з'єднання. Найбільше важить результат
/// запису гри (кадри, знімки, потоки): 10 хв на високому FPS — десятки МБ JSON.
pub const MAX_FRAME: u32 = 128 * 1024 * 1024;

/// Помилки протоколу.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    /// Помилка читання чи запису.
    #[error("канал: {0}")]
    Io(#[from] std::io::Error),
    /// Кадр більший за [`MAX_FRAME`].
    #[error("кадр завеликий: {0} байт")]
    TooLarge(u32),
    /// Кадр не розібрався (чужий формат або невідома команда).
    #[error("некоректне повідомлення: {0}")]
    Json(#[from] serde_json::Error),
}

/// Перше повідомлення кожної сторони.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    /// Версія протоколу ([`PROTOCOL`]).
    pub protocol: u32,
    /// Версія програми.
    pub version: String,
}

impl Hello {
    /// Привітання цієї збірки.
    pub fn ours() -> Self {
        Self {
            protocol: PROTOCOL,
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

/// Команди служби (закритий перелік).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    /// Перевірка зв'язку.
    Ping,
    /// Датчики CPU через PawnIO: температури й частоти шини пам'яті (лише читання).
    Sensors,
    /// Лише температура CPU (часті заміри).
    CpuTemp,
    /// Почати запис гри (`spec` — `optimax_backend::record::RecordSpec`).
    RecordStart {
        /// Що записувати.
        spec: serde_json::Value,
    },
    /// Опитування запису: інтерфейс передає активне вікно й натискання клавіші.
    RecordPoll {
        /// PID процесу з активним вікном.
        foreground_pid: Option<u32>,
        /// Гарячу клавішу натиснуто з минулого опитування.
        hotkey: bool,
    },
    /// Зупинити запис.
    RecordCancel,
    /// Почати стеження («Щойно лагнуло?», чорна скринька; `spec` — `optimax_backend::watch::WatchSpec`).
    WatchStart {
        /// Що стежити.
        spec: serde_json::Value,
    },
    /// Опитування стеження.
    WatchPoll {
        /// PID процесу з активним вікном.
        foreground_pid: Option<u32>,
        /// Ctrl+Shift+F11 натиснуто з минулого опитування.
        hotkey: bool,
        /// Кнопка «зберегти останні секунди».
        capture: bool,
    },
    /// Зупинити стеження.
    WatchStop,
    /// Почати збір живих даних оверлею.
    LiveStart,
    /// Опитування оверлею.
    LivePoll {
        /// PID процесу з активним вікном (без самої програми).
        foreground_pid: Option<u32>,
    },
    /// Зупинити збір оверлею.
    LiveStop,
    /// План зміни (`desired` — `optimax_tweaks::Desired`: твік із каталогу й параметр). Служба
    /// перевіряє все сама й запам'ятовує план; у відповіді — його номер і що зміниться.
    TweakPlan {
        /// Бажаний стан.
        desired: serde_json::Value,
    },
    /// Застосувати план, складений службою (номер з відповіді на [`Command::TweakPlan`]).
    TweakApply {
        /// Номер плану.
        plan_id: u64,
        /// Користувач погодився продовжити без точки відновлення.
        allow_without_restore_point: bool,
        /// Номер сесії програми (новий запуск — нова точка відновлення перед першою зміною).
        session: u64,
    },
    /// Очистити standby-список пам'яті (режим гри).
    PurgeStandby,
    /// Журнал змін.
    JournalEntries,
    /// Відкотити записи журналу (від найновішого).
    JournalRollback {
        /// Номери записів.
        ids: Vec<u64>,
    },
    /// Відкотити всі діючі зміни.
    JournalRollbackAll,
    /// Довести до кінця записи, що лишилися після збою.
    JournalRecover,
}

/// Запит клієнта.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    /// Номер запиту (повертається у відповіді).
    pub id: u64,
    /// Команда.
    #[serde(flatten)]
    pub command: Command,
}

/// Відповідь на [`Command::Ping`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Pong {
    /// Версія служби.
    pub version: String,
    /// Скільки служба працює, с.
    pub uptime_s: u64,
}

/// Результат команди.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reply {
    /// На `ping`.
    Pong(Pong),
    /// Результат команди (тип знає той, хто її викликав).
    Json(serde_json::Value),
    /// Служба відмовила або команда не вдалась.
    Error(String),
}

/// Відповідь служби.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    /// Номер запиту.
    pub id: u64,
    /// Результат.
    pub reply: Reply,
}

/// Пише один кадр.
pub fn write_frame<T: Serialize>(w: &mut impl Write, msg: &T) -> Result<(), ProtocolError> {
    let body = serde_json::to_vec(msg)?;
    let len = u32::try_from(body.len()).map_err(|_| ProtocolError::TooLarge(u32::MAX))?;
    if len > MAX_FRAME {
        return Err(ProtocolError::TooLarge(len));
    }
    w.write_all(&len.to_le_bytes())?;
    w.write_all(&body)?;
    w.flush()?;
    Ok(())
}

/// Читає один кадр. Довжина перевіряється до виділення пам'яті.
pub fn read_frame<T: DeserializeOwned>(r: &mut impl Read) -> Result<T, ProtocolError> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len);
    if len > MAX_FRAME {
        return Err(ProtocolError::TooLarge(len));
    }
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body)?;
    Ok(serde_json::from_slice(&body)?)
}

/// Чи підходить привітання другої сторони (версія протоколу збігається).
pub fn compatible(theirs: &Hello) -> bool {
    theirs.protocol == PROTOCOL
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn frames_roundtrip() {
        let mut buf = Vec::new();
        write_frame(&mut buf, &Hello::ours()).expect("hello");
        let req = Request {
            id: 7,
            command: Command::Ping,
        };
        write_frame(&mut buf, &req).expect("req");
        let mut r = Cursor::new(buf);
        assert_eq!(read_frame::<Hello>(&mut r).expect("hello"), Hello::ours());
        assert_eq!(read_frame::<Request>(&mut r).expect("req"), req);
    }

    #[test]
    fn request_json_is_flat_with_cmd_tag() {
        let v = serde_json::to_value(Request {
            id: 1,
            command: Command::Ping,
        })
        .expect("json");
        assert_eq!(v, serde_json::json!({ "id": 1, "cmd": "ping" }));
    }

    #[test]
    fn unknown_command_is_rejected() {
        let mut buf = Vec::new();
        write_frame(
            &mut buf,
            &serde_json::json!({ "id": 1, "cmd": "delete_everything" }),
        )
        .expect("кадр");
        assert!(matches!(
            read_frame::<Request>(&mut Cursor::new(buf)),
            Err(ProtocolError::Json(_))
        ));
    }

    #[test]
    fn oversized_frame_is_rejected_before_allocation() {
        let mut buf = (MAX_FRAME + 1).to_le_bytes().to_vec();
        buf.extend([0u8; 8]);
        assert!(matches!(
            read_frame::<Hello>(&mut Cursor::new(buf)),
            Err(ProtocolError::TooLarge(_))
        ));
    }

    #[test]
    fn sensor_commands_are_plain_tags() {
        let v = serde_json::to_value(Request {
            id: 2,
            command: Command::CpuTemp,
        })
        .expect("json");
        assert_eq!(v, serde_json::json!({ "id": 2, "cmd": "cpu_temp" }));
    }

    #[test]
    fn record_poll_carries_session_inputs() {
        let v = serde_json::to_value(Request {
            id: 3,
            command: Command::RecordPoll {
                foreground_pid: Some(20040),
                hotkey: true,
            },
        })
        .expect("json");
        assert_eq!(
            v,
            serde_json::json!({ "id": 3, "cmd": "record_poll", "foreground_pid": 20040, "hotkey": true })
        );
    }

    #[test]
    fn only_same_protocol_is_compatible() {
        assert!(compatible(&Hello::ours()));
        assert!(!compatible(&Hello {
            protocol: PROTOCOL + 1,
            version: "9.9.9".into(),
        }));
    }
}
