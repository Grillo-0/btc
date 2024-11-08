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

pub fn send_message(stream: &mut TcpStream, msg: BitcoinMsg) -> std::io::Result<()> {
    let blob = msg.to_blob();
    stream.write_all(&blob)?;
    Ok(())
}

pub fn read_message(stream: &mut TcpStream) -> std::io::Result<BitcoinMsg> {
    let mut header = vec![0; 24];
    stream.peek(&mut header)?;
    let header = BitcoinHeader::from_blob(&mut Scanner::new(header));

    let mut msg = vec![0; 24 + header.size as usize];
    stream.read_exact(&mut msg)?;

    let msg = BitcoinMsg::from_blob(&mut Scanner::new(msg));

    Ok(msg)
}

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

fn bitcoin_handling(rx: Receiver<ClientCommand>, tx: Sender<LogMsg>) -> std::io::Result<()> {
    let mut stream = None;

    for cmd in rx.iter() {
        let ClientCommand::SendBtcMsg(btc_msg) = cmd;

        if let BitcoinPayload::Version(ref ver) = btc_msg.payload {
            let mut s = TcpStream::connect(ver.remote.addr)?;

            send_message(&mut s, btc_msg)?;

            if let BitcoinPayload::Version(_) = read_message(&mut s)?.payload {
            } else {
                panic!()
            }

            if let BitcoinPayload::VerAck = read_message(&mut s)?.payload {
            } else {
                panic!();
            }

            send_message(&mut s, BitcoinMsg::verack())?;

            stream = Some(s);

            break;
        }
    }

    let mut stream = stream.unwrap();

    tx.send(LogMsg::info(format!(
        "Connected to address {}",
        stream.peer_addr().unwrap()
    )))
    .unwrap();

    stream.set_read_timeout(Some(Duration::from_millis(100)))?;

    loop {
        for cmd in rx.try_iter() {
            let ClientCommand::SendBtcMsg(btc_msg) = cmd;

            match btc_msg.payload {
                BitcoinPayload::Version(_) => {
                    tx.send(LogMsg::err("Already connected!")).unwrap();
                    continue;
                }
                BitcoinPayload::Ping(x) => {
                    tx.send(LogMsg::info(format!("Sending ping with value {x}")))
                        .unwrap();
                }
                BitcoinPayload::GetAddr => {
                    tx.send(LogMsg::info("Sending getaddr command")).unwrap();
                }
                _ => tx
                    .send(LogMsg::warn(format!("Sending {btc_msg:?}")))
                    .unwrap(),
            }

            send_message(&mut stream, btc_msg)?;
        }

        let msg = read_message(&mut stream);

        if let Err(e) = msg {
            match e.kind() {
                io::ErrorKind::WouldBlock => (),
                io::ErrorKind::TimedOut => (),
                _ => tx
                    .send(LogMsg::err(format!("Failed to read Message: {e}")))
                    .unwrap(),
            };

            continue;
        }

        let msg = msg.unwrap();

        match msg.payload {
            BitcoinPayload::Inv(p) => {
                tx.send(LogMsg::info(format!(
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
                    tx.send(LogMsg::info(send_str)).unwrap();
                }
            }
            BitcoinPayload::Ping(x) => {
                send_message(&mut stream, BitcoinMsg::pong(x))?;
            }
            BitcoinPayload::Pong(x) => {
                tx.send(LogMsg::info(format!("Received pong with value {x}")))
                    .unwrap();
            }
            BitcoinPayload::Addr(addrs) => {
                tx.send(LogMsg::info(format!(
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
                    tx.send(LogMsg::info(format!(
                        "addr: {}, timestamp: {}h{}m{}s",
                        addr.addr.addr,
                        time_since / 3600,
                        (time_since % 3600) / 60,
                        time_since % 60,
                    )))
                    .unwrap();
                }
            }
            _ => tx
                .send(LogMsg::warn(format!("Could not handle message {msg:?}")))
                .unwrap(),
        };
    }
}

const COMMAND_AREA_ROWS: u16 = 2;

fn main() -> std::io::Result<()> {
    let (log_tx, rx) = mpsc::channel();

    let (tx, rx_) = mpsc::channel();

    let log_tx_clone = log_tx.clone();
    let _handle = thread::spawn(move || bitcoin_handling(rx_, log_tx_clone));

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
