use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::thread::sleep;
use std::time::{Duration};

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

fn main() -> std::io::Result<()> {
    let mut stream = TcpStream::connect("203.11.72.155:8333")?;

    let version = BitcoinMsg::version(
        NetAddr {
            services: Default::default(),
            addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8333),
        },
        NetAddr {
            services: Default::default(),
            addr: stream.peer_addr()?,
        },
        "my bitcoin client".to_string(),
        69,
        0,
        true,
    );

    send_message(&mut stream, version)?;

    if let BitcoinPayload::Version(v) = read_message(&mut stream)?.payload {
        println!("{:#?}", v)
    } else {
        panic!()
    }

    if let BitcoinPayload::VerAck = read_message(&mut stream)?.payload {
    } else {
        panic!();
    }

    send_message(&mut stream, BitcoinMsg::verack())?;

    send_message(&mut stream, BitcoinMsg::getaddr())?;

    loop {
        let mut msg = read_message(&mut stream);
        while let Err(e) = msg {
            println!("Failed to read Message: {e}");
            msg = read_message(&mut stream);
        }

        let msg = msg.unwrap();

        match msg.payload {
            BitcoinPayload::Inv(p) => p.inventory.iter().for_each(|inv| {
                print!("{:?}: ", inv.kind);
                for x in inv.hash.iter().rev() {
                    print!("{x:02x}");
                }
                print!("\n");
            }),
            BitcoinPayload::Ping(x) => {
                send_message(&mut stream, BitcoinMsg::pong(x))?;
            }
            BitcoinPayload::Addr(x) => {
                println!("{:#?} nodes connected", x.addr_list.len());
                println!("{:#?}", x);
            }
            _ => println!("Message not covered: {msg:?}"),
        };
        sleep(Duration::from_secs(2));
    }
}
