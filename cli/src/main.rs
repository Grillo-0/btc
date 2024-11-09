use std::fmt::Write as _;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::result;
use std::str::FromStr;
use std::sync::mpsc::Sender;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, SystemTime};
use std::thread;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal;
use crossterm::ExecutableCommand;
use crossterm::{cursor, style, QueueableCommand};

use btc_lib::*;

#[derive(Debug)]
enum ErrorKind {
    IoErr(io::Error),
    NotConnected,
    ProtocolErr,
}

#[derive(Debug)]
struct Error {
    kind: ErrorKind,
    msg: Option<String>,
}

impl Error {
    fn new(kind: ErrorKind) -> Error {
        Error { kind, msg: None }
    }

    fn with_msg(kind: ErrorKind, msg: impl ToString) -> Error {
        Error {
            kind,
            msg: Some(msg.to_string()),
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::new(ErrorKind::IoErr(e))
    }
}

type Result<T> = result::Result<T, Error>;

enum LogMsgKind {
    Info,
    Warn,
    Error,
}

struct LogMsg {
    kind: LogMsgKind,
    msg: String,
}

impl LogMsg {
    fn info(msg: impl ToString) -> LogMsg {
        LogMsg {
            kind: LogMsgKind::Info,
            msg: msg.to_string(),
        }
    }

    fn warn(msg: impl ToString) -> LogMsg {
        LogMsg {
            kind: LogMsgKind::Warn,
            msg: msg.to_string(),
        }
    }

    fn err(msg: impl ToString) -> LogMsg {
        LogMsg {
            kind: LogMsgKind::Error,
            msg: msg.to_string(),
        }
    }
}

enum ClientCommand {
    SendBtcMsg(BitcoinMsg),
    Connect(SocketAddr),
    Disconnect,
}

struct Client {
    stream: Option<TcpStream>,
    log_tx: Sender<LogMsg>,
}

impl Client {
    fn send_msg(&mut self, msg: BitcoinMsg) -> Result<()> {
        if let Some(stream) = &mut self.stream {
            let blob = msg.to_blob();
            stream.write_all(&blob)?;
            Ok(())
        } else {
            Err(Error::with_msg(
                ErrorKind::NotConnected,
                format!("Could not send message {:#?}, client not connected", msg),
            ))
        }
    }

    fn read_msg(&mut self) -> Result<BitcoinMsg> {
        if let Some(stream) = &mut self.stream {
            let mut header = vec![0; 24];
            stream.peek(&mut header)?;
            let header = BitcoinHeader::from_blob(&mut Scanner::new(header));

            let mut msg = vec![0; 24 + header.size as usize];
            stream.read_exact(&mut msg)?;

            let msg = BitcoinMsg::from_blob(&mut Scanner::new(msg));
            Ok(msg)
        } else {
            Err(Error::with_msg(
                ErrorKind::NotConnected,
                "Could not receive message, client not connected",
            ))
        }
    }

    fn handle_cmds(&mut self, cmd: ClientCommand) -> Result<()> {
        match cmd {
            ClientCommand::SendBtcMsg(btc_msg) => self.send_msg_cmd(btc_msg)?,
            ClientCommand::Connect(addr) => self.connect(addr)?,
            ClientCommand::Disconnect => self.disconnect()?,
        }

        Ok(())
    }

    fn send_msg_cmd(&mut self, btc_msg: BitcoinMsg) -> Result<()> {
        match btc_msg.payload {
            BitcoinPayload::Version(_) => {
                self.log_tx.send(LogMsg::err("Already connected!")).unwrap();
            }
            BitcoinPayload::Ping(x) => {
                self.log_tx
                    .send(LogMsg::info(format!("Sending ping with value {x}")))
                    .unwrap();
            }
            BitcoinPayload::GetAddr => {
                self.log_tx
                    .send(LogMsg::info("Sending getaddr command"))
                    .unwrap();
            }
            _ => self
                .log_tx
                .send(LogMsg::warn(format!("Sending {btc_msg:?}")))
                .unwrap(),
        }

        self.send_msg(btc_msg)?;

        Ok(())
    }

    fn connect(&mut self, addr: SocketAddr) -> Result<()> {
        self.stream = TcpStream::connect(addr).ok();

        let msg = BitcoinMsg::version(
            NetAddr {
                services: Default::default(),
                addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8333),
            },
            NetAddr {
                services: Default::default(),
                addr,
            },
            "my bitcoin client".to_string(),
            69,
            0,
            true,
        );

        self.send_msg(msg)?;

        if let BitcoinPayload::Version(_) = self.read_msg()?.payload {
        } else {
            return Err(Error::new(ErrorKind::ProtocolErr));
        }

        if let BitcoinPayload::VerAck = self.read_msg()?.payload {
        } else {
            return Err(Error::new(ErrorKind::ProtocolErr));
        }

        self.send_msg(BitcoinMsg::verack())?;

        if let Some(stream) = &self.stream {
            stream.set_read_timeout(Some(Duration::from_millis(100)))?;

            self.log_tx
                .send(LogMsg::info(format!(
                    "Connected to address {}",
                    stream.peer_addr().unwrap()
                )))
                .unwrap();
        } else {
            unreachable!()
        }

        Ok(())
    }

    fn disconnect(&mut self) -> Result<()> {
        let addr = self.stream.as_ref().map(|s| s.peer_addr());
        self.stream = None;

        if let Some(addr) = addr {
            self.log_tx.send(LogMsg::info(format!("Disconnecting from {}", addr?))).unwrap();
        } else {
            self.log_tx.send(LogMsg::info("Already Disconnected")).unwrap();
        }

        Ok(())
    }
}

fn bitcoin_handling(mut client: Client, rx: Receiver<ClientCommand>) -> Result<()> {
    loop {
        for cmd in rx.try_iter() {
            if let Err(e) = client.handle_cmds(cmd) {
                if let ErrorKind::IoErr(_) = e.kind {
                    return Err(e);
                } else if let Some(msg) = e.msg {
                        client.log_tx.send(LogMsg::err(msg)).unwrap();
                }
            }
        }

        let msg = client.read_msg();

        if let Err(Error {
            kind: ErrorKind::NotConnected,
            ..
        }) = msg
        {
            continue;
        }

        if let Err(Error {
            kind: ErrorKind::IoErr(e),
            ..
        }) = msg
        {
            match e.kind() {
                io::ErrorKind::WouldBlock => (),
                io::ErrorKind::TimedOut => (),
                _ => client
                    .log_tx
                    .send(LogMsg::err(format!("Failed to read Message: {e}")))
                    .unwrap(),
            };

            continue;
        }

        let msg = msg.unwrap();

        match msg.payload {
            BitcoinPayload::Inv(p) => {
                client
                    .log_tx
                    .send(LogMsg::info(format!(
                        "Got {} new objects",
                        p.inventory.len()
                    )))
                    .unwrap();

                for inv in p.inventory.iter() {
                    let mut send_str = String::new();
                    write!(send_str, "{:?}: ", inv.kind).unwrap();
                    for x in inv.hash.iter().rev() {
                        write!(send_str, "{x:02x}").unwrap();
                    }
                    client.log_tx.send(LogMsg::info(send_str)).unwrap();
                }
            }
            BitcoinPayload::Ping(x) => {
                client.send_msg(BitcoinMsg::pong(x))?;
            }
            BitcoinPayload::Pong(x) => {
                client
                    .log_tx
                    .send(LogMsg::info(format!("Received pong with value {x}")))
                    .unwrap();
            }
            BitcoinPayload::Addr(addrs) => {
                client
                    .log_tx
                    .send(LogMsg::info(format!(
                        "Found {:#?} nodes",
                        addrs.addr_list.len()
                    )))
                    .unwrap();
                for addr in addrs.addr_list {
                    let time_since = SystemTime::now()
                        .duration_since(
                            SystemTime::UNIX_EPOCH + Duration::from_secs(addr.timestamp as u64),
                        )
                        .unwrap()
                        .as_secs();
                    client
                        .log_tx
                        .send(LogMsg::info(format!(
                            "addr: {}, timestamp: {}h{}m{}s",
                            addr.addr.addr,
                            time_since / 3600,
                            (time_since % 3600) / 60,
                            time_since % 60,
                        )))
                        .unwrap();
                }
            }
            _ => client
                .log_tx
                .send(LogMsg::warn(format!("Could not handle message {msg:?}")))
                .unwrap(),
        };
    }
}

const COMMAND_AREA_ROWS: u16 = 2;

fn main() -> std::io::Result<()> {
    let (log_tx, rx) = mpsc::channel();

    let (tx, cmd_rx) = mpsc::channel();

    let log_tx_clone = log_tx.clone();
    let _handle = thread::spawn(move || {
        bitcoin_handling(
            Client {
                stream: None,
                log_tx: log_tx_clone,
            },
            cmd_rx,
        )
    });

    let mut stdout = io::stdout();
    terminal::enable_raw_mode()?;

    let window_size = terminal::window_size()?;
    stdout
        .execute(terminal::Clear(terminal::ClearType::All))?
        .execute(cursor::MoveTo(0, window_size.rows - 1))?
        .execute(style::Print("> "))?;

    let mut command = String::new();
    let mut command_cursor_position = (2, window_size.rows - 1);
    let mut log_cursor_position = (0, 0);

    loop {
        if event::poll(Duration::from_secs(1))? {
            if let Event::Key(event) = event::read()? {
                if event == KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL) {
                    break;
                }

                if let KeyCode::Char(c) = event.code {
                    command.push(c);
                    stdout.queue(style::Print(c))?;
                }

                if event.code == KeyCode::Backspace && !command.is_empty() {
                    command.pop();
                    stdout
                        .queue(cursor::MoveLeft(1))?
                        .queue(style::Print(" "))?
                        .queue(cursor::MoveLeft(1))?;
                }

                if event.code == KeyCode::Enter {
                    let mut command_parsed = command.split_whitespace();

                    match &command_parsed.next() {
                        Some("connect") => {
                            if let Some(addr) = command_parsed.next() {
                                match SocketAddr::from_str(addr) {
                                    Ok(addr) => tx.send(ClientCommand::Connect(addr)).unwrap(),
                                    Err(e) => log_tx
                                        .send(LogMsg::err(format!(
                                            "Could not parse address \"{addr}\": {e}",
                                        )))
                                        .unwrap(),
                                }
                            } else {
                                log_tx.send(LogMsg::err("addr not provided!")).unwrap();
                            };
                        }
                        Some("disconnect") => tx
                            .send(ClientCommand::Disconnect)
                            .unwrap(),
                        Some("ping") => {
                            if let Some(value) = command_parsed.next() {
                                match value.parse() {
                                    Ok(value) => tx
                                        .send(ClientCommand::SendBtcMsg(BitcoinMsg::ping(value)))
                                        .unwrap(),
                                    Err(e) => log_tx
                                        .send(LogMsg::err(format!(
                                            "Could not parse value \"{value}\": {e}"
                                        )))
                                        .unwrap(),
                                }
                            } else {
                                log_tx
                                    .send(LogMsg::err("ping value not provided!"))
                                    .unwrap();
                            };
                        }
                        Some("getaddr") => tx
                            .send(ClientCommand::SendBtcMsg(BitcoinMsg::getaddr()))
                            .unwrap(),
                        Some(cmd) => log_tx
                            .send(LogMsg::err(format!("No command \"{cmd}\" no found")))
                            .unwrap(),
                        None => log_tx
                            .send(LogMsg::err("A command must be provided"))
                            .unwrap(),
                    }

                    stdout
                        .queue(cursor::MoveToColumn(2))?
                        .queue(style::Print(" ".repeat(command.len())))?
                        .queue(cursor::MoveToColumn(2))?;

                    command.clear();
                }

                command_cursor_position = cursor::position()?;
            }
        }

        stdout
            .queue(cursor::Hide)?
            .queue(cursor::MoveTo(log_cursor_position.0, log_cursor_position.1))?;

        for msg in rx.try_iter() {
            for msg_part in msg.msg.split('\n').filter(|s| !s.is_empty()) {
                match msg.kind {
                    LogMsgKind::Info => stdout
                        .queue(style::SetForegroundColor(style::Color::Blue))?
                        .queue(style::Print("INFO: "))?,
                    LogMsgKind::Warn => stdout
                        .queue(style::SetForegroundColor(style::Color::Yellow))?
                        .queue(style::Print("WARN: "))?,
                    LogMsgKind::Error => stdout
                        .queue(style::SetForegroundColor(style::Color::Red))?
                        .queue(style::Print("ERROR: "))?,
                }
                .queue(style::Print(msg_part))?
                .queue(style::ResetColor)?
                .queue(cursor::MoveToNextLine(1))?;

                if cursor::position()?.1 > window_size.rows - COMMAND_AREA_ROWS {
                    let dist = cursor::position()?.1 - (window_size.rows - COMMAND_AREA_ROWS);

                    stdout
                        .queue(cursor::SavePosition)?
                        .queue(cursor::MoveTo(0, window_size.rows - 1))?
                        .queue(style::Print(" ".repeat(command_cursor_position.0 as usize)))?
                        .queue(terminal::ScrollUp(dist))?
                        .queue(cursor::MoveTo(0, window_size.rows - 1))?
                        .queue(style::Print("> "))?
                        .queue(style::Print(command.clone()))?
                        .queue(cursor::RestorePosition)?
                        .queue(cursor::MoveToPreviousLine(dist))?;
                }
            }
        }

        log_cursor_position = cursor::position()?;

        stdout
            .queue(cursor::MoveTo(
                command_cursor_position.0,
                command_cursor_position.1,
            ))?
            .queue(cursor::Show)?;

        stdout.flush()?;
    }

    terminal::disable_raw_mode()?;
    Ok(())
}
