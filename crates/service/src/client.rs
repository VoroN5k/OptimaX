//! Клієнт служби: з'єднання, перевірка, що на тому кінці наш exe, привітання, запити.

use crate::pipe::{process_image, PipeStream};
use crate::protocol::{
    compatible, read_frame, write_frame, Command, Hello, Pong, Reply, Request, Response, PIPE_NAME,
};
use crate::{same_file, service_exe, ServiceError};

/// З'єднання зі службою після перевірки сервера й привітання.
#[derive(Debug)]
pub struct Client {
    pipe: PipeStream,
    next_id: u64,
    /// Привітання служби.
    pub server: Hello,
}

impl Client {
    /// З'єднується зі службою. Відмовляє, якщо канал створив процес без прав або не
    /// `%ProgramFiles%\OptimaX\OptimaX.exe` (хтось підставив свій канал), чи версія протоколу інша.
    pub fn connect() -> Result<Self, ServiceError> {
        let mut pipe = PipeStream::connect(PIPE_NAME)?;
        let server_pid = pipe.server_pid()?;
        if !pipe.owner_is_privileged()? {
            return Err(ServiceError::Untrusted(format!(
                "канал створив процес без прав адміністратора (PID {server_pid})"
            )));
        }
        // З правами — ще й шлях exe. Без прав процес SYSTEM не відкрити (доступ заборонено):
        // тоді достатньо власника каналу — підставити канал від SYSTEM може лише адміністратор.
        match process_image(server_pid) {
            Ok(server_exe) => {
                let trusted = service_exe().is_some_and(|ours| same_file(&server_exe, &ours));
                if !trusted {
                    return Err(ServiceError::Untrusted(server_exe.display().to_string()));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {}
            Err(e) => return Err(e.into()),
        }
        write_frame(&mut pipe, &Hello::ours())?;
        let server: Hello = read_frame(&mut pipe)?;
        if !compatible(&server) {
            return Err(ServiceError::Incompatible(server.version));
        }
        Ok(Self {
            pipe,
            next_id: 1,
            server,
        })
    }

    /// Виконує команду й повертає відповідь як є.
    pub fn call(&mut self, command: Command) -> Result<Reply, ServiceError> {
        let id = self.next_id;
        self.next_id += 1;
        write_frame(&mut self.pipe, &Request { id, command })?;
        let resp: Response = read_frame(&mut self.pipe)?;
        if resp.id != id {
            return Err(ServiceError::Refused(format!(
                "відповідь на чужий запит ({} замість {id})",
                resp.id
            )));
        }
        Ok(resp.reply)
    }

    /// Виконує команду, що відповідає JSON-значенням типу `T`.
    pub fn json<T: serde::de::DeserializeOwned>(
        &mut self,
        command: Command,
    ) -> Result<T, ServiceError> {
        match self.call(command)? {
            Reply::Json(v) => serde_json::from_value(v)
                .map_err(|e| ServiceError::Protocol(crate::protocol::ProtocolError::Json(e))),
            Reply::Error(e) => Err(ServiceError::Refused(e)),
            Reply::Pong(_) => Err(ServiceError::Refused("неочікувана відповідь".into())),
        }
    }

    /// Перевірка зв'язку.
    pub fn ping(&mut self) -> Result<Pong, ServiceError> {
        match self.call(Command::Ping)? {
            Reply::Pong(p) => Ok(p),
            Reply::Error(e) => Err(ServiceError::Refused(e)),
            Reply::Json(_) => Err(ServiceError::Refused("неочікувана відповідь".into())),
        }
    }
}
