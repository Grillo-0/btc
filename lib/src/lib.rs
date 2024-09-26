use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::panic;
use std::time::{Duration, SystemTime};

use sha2::Digest;
use sha2::Sha256;

use btc_lib_proc_macros::BitcoinType;

#[derive(Debug, Clone)]
pub struct Scanner {
    bytes: Vec<u8>,
    it: usize,
}

impl Scanner {
    pub fn new(bytes: Vec<u8>) -> Scanner {
        Scanner { bytes, it: 0 }
    }

    pub fn take(&mut self, amnt: usize) -> &[u8] {
        let ret = &self.bytes[self.it..(self.it + amnt)];
        self.it += amnt;
        ret
    }

    pub fn peek(&mut self, amnt: usize) -> &[u8] {
        &self.bytes[self.it..(self.it + amnt)]
    }
}

pub trait BitcoinType {
    fn to_blob(&self) -> Vec<u8>;
    fn from_blob(blob: &mut Scanner) -> Self;
}

#[derive(Debug, Clone)]
pub enum InventoryKind {
    Error,
    Tx,
    Block,
    FilteredBlock,
    CmpctBlock,
    WitnessTx,
    WitnessBlock,
    FilteredWitnessBlock,
}

#[derive(Debug, Clone)]
pub struct InventoryElement {
    pub kind: InventoryKind,
    pub hash: [u8; 32],
}

impl BitcoinType for InventoryElement {
    fn to_blob(&self) -> Vec<u8> {
        use InventoryKind::*;

        let kind_value: u32 = match self.kind {
            Error => 0x0,
            Tx => 0x1,
            Block => 0x2,
            FilteredBlock => 0x3,
            CmpctBlock => 0x4,
            WitnessTx => 0x40000001,
            WitnessBlock => 0x40000002,
            FilteredWitnessBlock => 0x40000003,
        };

        let mut ret = vec![];
        ret.extend(kind_value.to_blob());
        ret.extend(self.hash.to_vec());
        ret
    }

    fn from_blob(blob: &mut Scanner) -> Self {
        use InventoryKind::*;

        let kind = u32::from_blob(blob);

        let kind = match kind {
            0x0 => Error,
            0x1 => Tx,
            0x2 => Block,
            0x3 => FilteredBlock,
            0x4 => CmpctBlock,
            0x40000001 => WitnessTx,
            0x40000002 => WitnessBlock,
            0x40000003 => FilteredWitnessBlock,
            _ => panic!("no message type with code 0x{:x} ", kind),
        };

        InventoryElement {
            kind,
            hash: blob.take(32).try_into().unwrap(),
        }
    }
}

fn get_check_sum(src: &[u8]) -> Vec<u8> {
    Sha256::digest(Sha256::digest(src))[0..4].to_vec()
}

impl BitcoinType for u8 {
    fn to_blob(&self) -> Vec<u8> {
        vec![*self]
    }

    fn from_blob(blob: &mut Scanner) -> Self {
        blob.take(1)[0]
    }
}

impl BitcoinType for u16 {
    fn to_blob(&self) -> Vec<u8> {
        self.to_le_bytes().to_vec()
    }

    fn from_blob(blob: &mut Scanner) -> Self {
        Self::from_le_bytes(blob.take(2).try_into().unwrap())
    }
}

impl BitcoinType for u32 {
    fn to_blob(&self) -> Vec<u8> {
        self.to_le_bytes().to_vec()
    }

    fn from_blob(blob: &mut Scanner) -> Self {
        Self::from_le_bytes(blob.take(4).try_into().unwrap())
    }
}

impl BitcoinType for u64 {
    fn to_blob(&self) -> Vec<u8> {
        self.to_le_bytes().to_vec()
    }

    fn from_blob(blob: &mut Scanner) -> Self {
        Self::from_le_bytes(blob.take(8).try_into().unwrap())
    }
}

impl BitcoinType for bool {
    fn to_blob(&self) -> Vec<u8> {
        (*self as u8).to_blob()
    }

    fn from_blob(blob: &mut Scanner) -> Self {
        u8::from_blob(blob) != 0
    }
}

impl BitcoinType for usize {
    fn to_blob(&self) -> Vec<u8> {
        if *self < 0xfd {
            (*self as u8).to_le_bytes().to_vec()
        } else if *self <= 0xffff {
            let mut ret = vec![0xfd];
            ret.extend((*self as u16).to_le_bytes().to_vec());
            ret
        } else if *self <= 0xffff_ffff {
            let mut ret = vec![0xfe];
            ret.extend((*self as u32).to_le_bytes().to_vec());
            ret
        } else {
            let mut ret = vec![0xff];
            ret.extend((*self as u64).to_le_bytes().to_vec());
            ret
        }
    }

    fn from_blob(blob: &mut Scanner) -> Self {
        let first_byte = u8::from_blob(blob);
        match first_byte {
            0xff => u64::from_blob(blob) as usize,
            0xfe => u32::from_blob(blob) as usize,
            0xfd => u16::from_blob(blob) as usize,
            x => x as usize,
        }
    }
}

impl BitcoinType for String {
    fn to_blob(&self) -> Vec<u8> {
        let mut ret = vec![];
        ret.extend(self.len().to_blob());
        ret.extend(self.bytes());
        ret
    }

    fn from_blob(blob: &mut Scanner) -> Self {
        let len = usize::from_blob(blob);
        let str = blob.take(len);
        String::from_utf8_lossy(str).to_string()
    }
}

impl BitcoinType for SystemTime {
    fn to_blob(&self) -> Vec<u8> {
        let time = self.duration_since(SystemTime::UNIX_EPOCH).unwrap();
        time.as_secs().to_blob()
    }

    fn from_blob(blob: &mut Scanner) -> Self {
        let secs = u64::from_blob(blob);
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }
}

impl<T: BitcoinType, const N: usize> BitcoinType for [T; N] {
    fn to_blob(&self) -> Vec<u8> {
        self.iter().flat_map(|e| e.to_blob()).collect()
    }

    fn from_blob(blob: &mut Scanner) -> Self {
        let mut ret = vec![];
        for _ in 0..N {
            ret.push(T::from_blob(blob));
        }

        if let Ok(ret) = ret.try_into() {
            ret
        } else {
            unreachable!();
        }
    }
}

impl<T: BitcoinType> BitcoinType for Vec<T> {
    fn to_blob(&self) -> Vec<u8> {
        let mut ret = vec![];
        ret.extend(self.len().to_blob());
        for e in self {
            ret.extend(e.to_blob());
        }
        ret
    }

    fn from_blob(blob: &mut Scanner) -> Self {
        let count = usize::from_blob(blob);
        let mut vec = Vec::with_capacity(count);
        for _ in 0..count {
            vec.push(T::from_blob(blob));
        }
        vec
    }
}

#[derive(Debug, Clone, Default)]
pub struct Services {
    pub network: bool,
    pub getutxo: bool,
    pub bloom: bool,
    pub witness: bool,
    pub xthin: bool,
    pub compact_filters: bool,
    pub network_limited: bool,
}

impl BitcoinType for Services {
    fn from_blob(blob: &mut Scanner) -> Self {
        let bitfield = u64::from_blob(blob);

        Services {
            network: (bitfield >> 1) & 1 == 1,
            getutxo: (bitfield >> 2) & 1 == 1,
            bloom: (bitfield >> 3) & 1 == 1,
            witness: (bitfield >> 4) & 1 == 1,
            xthin: (bitfield >> 5) & 1 == 1,
            compact_filters: (bitfield >> 7) & 1 == 1,
            network_limited: (bitfield >> 10) & 1 == 1,
        }
    }

    fn to_blob(&self) -> Vec<u8> {
        let bitfield = (self.network as u64) << 1
            & (self.getutxo as u64) << 2
            & (self.bloom as u64) << 3
            & (self.witness as u64) << 4
            & (self.xthin as u64) << 5
            & (self.compact_filters as u64) << 7
            & (self.network_limited as u64) << 10;

        bitfield.to_blob()
    }
}

impl BitcoinType for SocketAddr {
    fn to_blob(&self) -> Vec<u8> {
        let mut res = match self.ip() {
            IpAddr::V4(ip) => ip.to_ipv6_mapped().octets().to_vec(),
            IpAddr::V6(ip) => ip.octets().to_vec(),
        };

        res.extend(self.port().to_be_bytes().to_vec());
        res
    }

    fn from_blob(blob: &mut Scanner) -> Self {
        let ip = Ipv6Addr::from(<&[u8] as TryInto<[u8; 16]>>::try_into(blob.take(16)).unwrap());
        let ip = if let Some(ipv4) = ip.to_ipv4_mapped() {
            IpAddr::V4(ipv4)
        } else {
            IpAddr::V6(ip)
        };

        let port = u16::from_be_bytes(blob.take(2).try_into().unwrap());
        SocketAddr::new(ip, port)
    }
}

#[derive(Debug, Clone, BitcoinType)]
pub struct NetAddr {
    pub services: Services,
    pub addr: SocketAddr,
}

#[derive(Debug, Clone, BitcoinType)]
pub struct Version {
    pub proto_ver: u32,
    pub services: Services,
    pub time: SystemTime,
    pub remote: NetAddr,
    pub local: NetAddr,
    pub nonce: u64,
    pub user_agent: String,
    pub last_block: u32,
    pub relay: bool,
}

#[derive(Debug, Clone, BitcoinType)]
pub struct SendCmpct {
    pub flag: bool,
    pub integer: u64,
}

#[derive(Debug, Clone, BitcoinType)]
pub struct FeeFilter {
    pub feerate: u64,
}

#[derive(Debug, Clone, BitcoinType)]
pub struct Inv {
    pub inventory: Vec<InventoryElement>,
}

#[derive(Debug, Clone, BitcoinType)]
pub struct AddrElement {
    pub timestamp: u32,
    pub addr: NetAddr,
}

#[derive(Debug, Clone, BitcoinType)]
pub struct Addr {
    pub addr_list: Vec<AddrElement>,
}

#[derive(Debug, Clone, BitcoinType)]
pub struct BitcoinHeader {
    pub magic: [u8; 4],
    pub command: [u8; 12],
    pub size: u32,
    pub check_sum: [u8; 4],
}

#[derive(Debug, Clone)]
pub enum BitcoinPayload {
    Version(Version),
    VerAck,
    SendHeaders,
    SendCmpct(SendCmpct),
    Ping(u64),
    Pong(u64),
    FeeFilter(FeeFilter),
    Inv(Inv),
    GetAddr,
    Addr(Addr),
}

#[derive(Debug, Clone)]
pub struct BitcoinMsg {
    pub payload: BitcoinPayload,
}

impl BitcoinType for BitcoinMsg {
    fn to_blob(&self) -> Vec<u8> {
        use BitcoinPayload::*;

        let mut blob = vec![0xf9, 0xbe, 0xb4, 0xd9]; // magic bytes

        let command = match self.payload {
            Version(_) => "version",
            VerAck => "verack",
            SendHeaders => "sendheaders",
            SendCmpct(_) => "sendcmpct",
            Ping(_) => "ping",
            Pong(_) => "pong",
            FeeFilter(_) => "feefilter",
            Inv(_) => "inv",
            GetAddr => "getaddr",
            Addr(_) => "addr",
        };

        let mut command = command.as_bytes().to_vec();
        command.resize(12, 0);

        blob.extend(command);

        let mut payload = vec![];
        match &self.payload {
            Version(p) => payload.extend(p.to_blob()),
            VerAck => {}
            SendHeaders => {}
            SendCmpct(p) => payload.extend(p.to_blob()),
            Ping(x) => payload.extend(x.to_blob()),
            Pong(x) => payload.extend(x.to_blob()),
            FeeFilter(p) => payload.extend(p.to_blob()),
            Inv(p) => payload.extend(p.to_blob()),
            GetAddr => {}
            Addr(p) => payload.extend(p.to_blob()),
        }

        let size = payload.len() as u32;
        let check_sum = if size != 0 {
            get_check_sum(&payload)
        } else {
            vec![0x5d, 0xf6, 0xe0, 0xe2]
        };

        blob.extend(size.to_le_bytes().to_vec());
        blob.extend(check_sum);
        blob.extend(payload);

        return blob;
    }

    fn from_blob(blob: &mut Scanner) -> Self {
        let header = BitcoinHeader::from_blob(blob);
        if header.magic != [0xf9, 0xbe, 0xb4, 0xd9] {
            panic!();
        }

        let mut command = header.command.to_vec();
        command.retain(|&x| x != 0);
        let command = std::str::from_utf8(&command).unwrap();

        let bulk = blob.peek(header.size as usize);

        if get_check_sum(bulk) != header.check_sum {
            panic!("Message is corrupted!");
        }

        let payload = match command {
            "version" => BitcoinPayload::Version(Version::from_blob(blob)),
            "verack" => BitcoinPayload::VerAck,
            "sendheaders" => BitcoinPayload::SendHeaders,
            "sendcmpct" => BitcoinPayload::SendCmpct(SendCmpct::from_blob(blob)),
            "ping" => BitcoinPayload::Ping(u64::from_blob(blob)),
            "pong" => BitcoinPayload::Pong(u64::from_blob(blob)),
            "feefilter" => BitcoinPayload::FeeFilter(FeeFilter::from_blob(blob)),
            "inv" => BitcoinPayload::Inv(Inv::from_blob(blob)),
            "getaddr" => BitcoinPayload::GetAddr,
            "addr" => BitcoinPayload::Addr(Addr::from_blob(blob)),
            _ => panic!("command {command} is not supported!"),
        };

        BitcoinMsg { payload }
    }
}

impl BitcoinMsg {
    pub fn getaddr() -> BitcoinMsg {
        BitcoinMsg {
            payload: BitcoinPayload::GetAddr,
        }
    }

    pub fn ping(nonce: u64) -> BitcoinMsg {
        BitcoinMsg {
            payload: BitcoinPayload::Ping(nonce),
        }
    }

    pub fn pong(nonce: u64) -> BitcoinMsg {
        BitcoinMsg {
            payload: BitcoinPayload::Pong(nonce),
        }
    }

    pub fn verack() -> BitcoinMsg {
        BitcoinMsg {
            payload: BitcoinPayload::VerAck,
        }
    }

    pub fn version(
        local: NetAddr,
        remote: NetAddr,
        user_agent: String,
        nonce: u64,
        last_block: u32,
        relay: bool,
    ) -> BitcoinMsg {
        BitcoinMsg {
            payload: BitcoinPayload::Version(Version {
                proto_ver: 70014,
                time: SystemTime::now(),
                services: local.services.clone(),
                remote,
                local,
                nonce,
                user_agent,
                last_block,
                relay,
            }),
        }
    }
}
