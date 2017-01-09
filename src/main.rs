extern crate byteorder;
extern crate clap;
extern crate futures;
extern crate tokio_core;

use byteorder::{BigEndian, ByteOrder};
use clap::{Arg, App};
use futures::future;
use futures::{Future, Sink};
use futures::stream::{Stream};
use std::io::{self, Error, ErrorKind};
use std::collections::{HashMap};
use std::net::{SocketAddr, Shutdown};
use tokio_core::io::{Io, Codec, EasyBuf, Framed};
use tokio_core::net::{TcpListener, TcpStream};
use tokio_core::reactor::{Core, Handle};
use std::rc::Rc;

const CONNECT: u8 = 1;
const DATA: u8 = 2;
const ACK: u8 = 3;
const CLOSE: u8 = 4;

pub enum DataFrame {
    /* an id of 0 means that we are making a new connection,
     * on request the seq is the last packet received from
     * server, on responce is is the last packet received by
     * server. */
    Connect {id: u32, seq: u32},
    Data {seq: u32, data: Vec<u8>,},
    Ack {seq: u32 },
    Close,
}

pub struct DataFrameCodec;

fn write_u32(u: u32, into: &mut Vec<u8>) {
    let a: &mut [u8; 4] = &mut [0; 4];
    BigEndian::write_u32(a, u);
    into.extend_from_slice(a);
}

impl Codec for DataFrameCodec {
    type In = DataFrame;
    type Out = DataFrame;

    fn decode(&mut self, buf: &mut EasyBuf) -> Result<Option<DataFrame>, io::Error> {
        let buf_len: usize = buf.len();

        if buf_len == 0 {
            return Ok(None);
        }

        let slice = buf.as_slice();

        match slice[0] {
            CONNECT => {
                if buf_len < 9 {
                    Ok(None) // need type + 2 u32s
                } else {
                    Ok(Some(DataFrame::Connect {
                        id: BigEndian::read_u32(&slice[1..4]),
                        seq: BigEndian::read_u32(&slice[5..7]),
                    }))
                }
            },
            DATA => {
                if buf_len < 9 {
                    Ok(None) // need type + 2 u32s
                } else {
                    let seq = BigEndian::read_u32(&slice[1..4]);
                    let len = BigEndian::read_u32(&slice[5..7]);
                    if (buf_len as u32) < 9 + len {
                        Ok(None)
                    } else {
                        let data = slice[8..(9+len as usize)].to_vec();
                        Ok(Some(DataFrame::Data {
                            seq: seq,
                            data: data,
                        }))
                    }
                }
            },
            ACK => {
                if buf_len < 5 {
                    Ok(None) // need type + 1 u32
                } else {
                    Ok(Some(DataFrame::Ack {
                        seq: BigEndian::read_u32(&slice[1..4]),
                    }))
                }
            },
            CLOSE => Ok(Some(DataFrame::Close)),
            _ => Err(Error::new(ErrorKind::InvalidInput, "Unknown type")),
        }
    }

    fn encode(&mut self, item: DataFrame, into: &mut Vec<u8>) -> io::Result<()> {
        match item {
            DataFrame::Connect{id, seq} => {
                into.push(CONNECT);
                write_u32(id, into);
                write_u32(seq, into)
            },
            DataFrame::Data{seq, data} => {
                into.push(DATA);
                let data_len = data.len();
                write_u32(seq, into);
                write_u32(data_len as u32, into);
                into.extend(data)
            },
            DataFrame::Ack{seq} => {
                into.push(ACK);
                write_u32(seq, into);
            },
            DataFrame::Close => {
                into.push(CLOSE)
            },
        }

        Ok(())
    }
}
type BoxedDataFrameSink = Box<Sink<SinkItem=DataFrame, SinkError=io::Error>>;
type BoxedDataFrameStream = Box<Stream<Item=DataFrame, Error=io::Error>>;

struct Connection<C: Codec + 'static>  {
    id: u32,
    framed: Rc<Framed<TcpStream, C>>,
}

struct TunnelConnection {
    sink1: BoxedDataFrameSink,
    stream1: BoxedDataFrameStream,
    handle: Rc<Handle>,
}

struct ConnectionRegistry<C: Codec + 'static> {
    connections: HashMap<u32, Rc<Connection<C>>>,
    next_key: u32,
}

impl<C: Codec> ConnectionRegistry<C> {
    fn register(&mut self, tcp_stream: TcpStream, codec: C) -> Rc<Connection<C>> {
        // id 0 is special (indicating no connection, so make sure to skip it
        while self.next_key != 0 && self.connections.contains_key(&self.next_key) {
            self.next_key += 1
        }
        
        let key = self.next_key;
        self.next_key += 1;

        let cnxn = Connection {
            id: key,
            framed: Rc::new(tcp_stream.framed(codec)),
        };

        let rc_cnxn = Rc::new(cnxn);
        self.connections.insert(key, rc_cnxn.clone());
        rc_cnxn
    }

    fn get_connection(&self, id: u32) -> Option<&Rc<Connection<C>>> {
        self.connections.get(&id)
    }

    fn close(&mut self, id: u32) {
        if let Some(cnxn) = self.connections.remove(&id) {
            cnxn.framed.get_ref().shutdown(Shutdown::Both);
        }
    }
}

impl TunnelConnection {
    fn new(sink1: BoxedDataFrameSink,
           stream1: BoxedDataFrameStream,
           handle: Rc<Handle>,) -> TunnelConnection {
        TunnelConnection {
            sink1: sink1,
            stream1: stream1,
            handle: handle,
        }
    }

    fn connect_sink_stream(self, cnxn: TcpStream) -> Result<(), io::Error> {
            let sink1 = self.sink1;
            let stream1 = self.stream1;
            let (sink2, stream2) = cnxn.framed(DataFrameCodec).split();
            self.handle.spawn(stream1.forward(sink2).then(|_| {println!("Sender closed"); Ok(())}));
            self.handle.spawn(stream2.forward(sink1).then(|_| {println!("Reciever closed"); Ok(())}));
            Ok(())
    }

    fn connect(self, addr: SocketAddr) {
        let h = &self.handle.clone();
        h.spawn(TcpStream::connect(&addr, h).and_then(move |cnxn| {
            self.connect_sink_stream(cnxn)
        }).then(|_| {println!("Connection finished"); Ok(())}));
    }
}

fn main() {
    let matches = App::new("tunnel one local port to remote port")
            .version("1.0")
            .arg(Arg::with_name("localPort")
                .short("p")
                .long("port")
                .value_name("PORT")
                .required(true)
                .help("local port to listen on"))
            .arg(Arg::with_name("remoteAddr")
                .short("r")
                .long("remoteAddr")
                .value_name("REMOTE ADDRESS")
                .required(true)
                .help("remote address to connect to"))
            .get_matches();

    let local_port = matches.value_of("localPort").unwrap().parse().unwrap();
    println!("Evaluating {}", matches.value_of("remoteAddr").unwrap());
    let remote_addr = matches.value_of("remoteAddr").unwrap().parse().unwrap();

    let mut core = Core::new().unwrap();
    let handle = Rc::new(core.handle());
    let acceptor = TcpListener::bind(&local_port, &handle).and_then(move |listener| {
        Ok(listener.incoming().for_each(move |(cnxn, addr)| {
                println!("got connection from {}", addr);
                let (sink, stream) = cnxn.framed(DataFrameCodec).split();
                let t = TunnelConnection::new(Box::new(sink), Box::new(stream), handle.clone());
                t.connect(remote_addr);
                Ok(())
        }))
    }).unwrap();
    core.run(acceptor).unwrap();
}
