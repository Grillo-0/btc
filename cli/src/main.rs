use std::fmt::Write as _;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
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
}

struct Client {
    stream: Option<TcpStream>,
    log_tx: Sender<LogMsg>,
}

impl Client {
    fn send_msg(&mut self, msg: BitcoinMsg) -> std::io::Result<()> {
        if let Some(stream) = &mut self.stream {
            let blob = msg.to_blob();
            stream.write_all(&blob)?;
        } else {
            self.log_tx
                .send(LogMsg::err(format!(
                    "Could not send message {:#?}, client not connected",
                    msg
                )))
                .unwrap();
        }

        Ok(())
    }

    fn read_msg(&mut self) -> std::io::Result<Option<BitcoinMsg>> {
        if let Some(stream) = &mut self.stream {
            let mut header = vec![0; 24];
            stream.peek(&mut header)?;
            let header = BitcoinHeader::from_blob(&mut Scanner::new(header));

            let mut msg = vec![0; 24 + header.size as usize];
            stream.read_exact(&mut msg)?;

            let msg = BitcoinMsg::from_blob(&mut Scanner::new(msg));
            Ok(Some(msg))
        } else {
            self.log_tx
                .send(LogMsg::err(
                    "Could not receive message, client not connected",
                ))
                .unwrap();
            Ok(None)
        }
    }
}

fn bitcoin_handling(mut client: Client, rx: Receiver<ClientCommand>) -> std::io::Result<()> {
    for cmd in rx.iter() {
        let ClientCommand::SendBtcMsg(btc_msg) = cmd;

        if let BitcoinPayload::Version(ref ver) = btc_msg.payload {
            client.stream = Some(TcpStream::connect(ver.remote.addr)?);

            client.send_msg(btc_msg)?;

            if let Some(BitcoinMsg {
                payload: BitcoinPayload::Version(_),
            }) = client.read_msg()?
            {
            } else {
                panic!()
            }

            if let Some(BitcoinMsg {
                payload: BitcoinPayload::VerAck,
            }) = client.read_msg()?
            {
            } else {
                panic!();
            }

            client.send_msg(BitcoinMsg::verack())?;

            break;
        }
    }

    if let Some(stream) = &client.stream {
        client
            .log_tx
            .send(LogMsg::info(format!(
                "Connected to address {}",
                stream.peer_addr().unwrap()
            )))
            .unwrap();
        stream.set_read_timeout(Some(Duration::from_millis(100)))?;
    }

    loop {
        for cmd in rx.try_iter() {
            let ClientCommand::SendBtcMsg(btc_msg) = cmd;

            match btc_msg.payload {
                BitcoinPayload::Version(_) => {
                    client
                        .log_tx
                        .send(LogMsg::err("Already connected!"))
                        .unwrap();
                    continue;
                }
                BitcoinPayload::Ping(x) => {
                    client
                        .log_tx
                        .send(LogMsg::info(format!("Sending ping with value {x}")))
                        .unwrap();
                }
                BitcoinPayload::GetAddr => {
                    client
                        .log_tx
                        .send(LogMsg::info("Sending getaddr command"))
                        .unwrap();
                }
                _ => client
                    .log_tx
                    .send(LogMsg::warn(format!("Sending {btc_msg:?}")))
                    .unwrap(),
            }

            client.send_msg(btc_msg)?;
        }

        let msg = client.read_msg();

        if let Err(e) = msg {
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

        let msg = msg.unwrap().unwrap();

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
                                    Ok(addr) => {
                                        let msg = BitcoinMsg::version(
                                            NetAddr {
                                                services: Default::default(),
                                                addr: SocketAddr::new(
                                                    IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
                                                    8333,
                                                ),
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

                                        tx.send(ClientCommand::SendBtcMsg(msg)).unwrap();
                                    }
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
