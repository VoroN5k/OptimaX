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
    /// З'єднується зі службою. Відмовляє, якщо канал тримає не `%ProgramFiles%\OptimaX\OptimaX.exe`
    /// (хтось підставив свій канал) або версія протоколу інша.
    pub fn connect() -> Result<Self, ServiceError> {
        let mut pipe = PipeStream::connect(PIPE_NAME)?;
        let server_exe = process_image(pipe.server_pid()?)?;
        let trusted = service_exe().is_some_and(|ours| same_file(&server_exe, &ours));
        if !trusted {
            return Err(ServiceError::Untrusted(server_exe.display().to_string()));
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

    fn call(&mut self, command: Command) -> Result<Reply, ServiceError> {
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

    /// Перевірка зв'язку.
    pub fn ping(&mut self) -> Result<Pong, ServiceError> {
        match self.call(Command::Ping)? {
            Reply::Pong(p) => Ok(p),
            Reply::Error(e) => Err(ServiceError::Refused(e)),
        }
    }
}
