extern crate byteorder;
extern crate clap;
extern crate futures;
extern crate tokio_core;

use clap::{Arg, App, SubCommand};
use futures::{Future, Sink};
use futures::stream::Stream;
use std::io::{self, Error, ErrorKind};
use std::collections::HashMap;
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6, Shutdown};
use tokio_core::io::{Io, Codec, Framed};
use tokio_core::net::{TcpListener, TcpStream};
use tokio_core::reactor::{Core, Handle};
use std::rc::Rc;

mod punnel_proto;
use punnel_proto::{PunnelFrame, PunnelFrameCodec};

type BoxedPunnelFrameSink = Box<Sink<SinkItem = PunnelFrame, SinkError = io::Error>>;
type BoxedPunnelFrameStream = Box<Stream<Item = PunnelFrame, Error = io::Error>>;

struct Connection {
    id: u32,
    framed: Rc<Framed<TcpStream, PunnelFrameCodec>>,
}

struct ConnectedConnection {
    cnxn: Connection,
    tunnel: Rc<Framed<TcpStream, PunnelFrameCodec>>,
    handle: Rc<Handle>,
}

struct ConnectionRegistry {
    connections: HashMap<u32, Rc<Connection>>,
    next_key: u32,
}

impl ConnectionRegistry {
    // for use by the server to add a connection
    fn register(&mut self, tcp_stream: TcpStream) -> Rc<Connection> {
        // id 0 is special (indicating no connection, so make sure to skip it
        while self.next_key != 0 && self.connections.contains_key(&self.next_key) {
            self.next_key += 1
        }

        let key = self.next_key;
        self.next_key += 1;
        self.add(key, tcp_stream)
    }

    // for use by the client to add a connection
    fn add(&mut self, id: u32, tcp_stream: TcpStream) -> Rc<Connection> {
        let cnxn = Connection {
            id: id,
            framed: Rc::new(tcp_stream.framed(PunnelFrameCodec)),
        };

        let rc_cnxn = Rc::new(cnxn);
        self.connections.insert(id, rc_cnxn.clone());
        rc_cnxn
    }

    fn get_connection(&self, id: u32) -> Option<&Rc<Connection>> {
        self.connections.get(&id)
    }

    fn close(&mut self, id: u32) {
        if let Some(cnxn) = self.connections.remove(&id) {
            cnxn.framed.get_ref().shutdown(Shutdown::Both);
        }
    }
}

impl ConnectedConnection {
    fn new(cnxn: Connection,
           tunnel: Rc<Framed<TcpStream, PunnelFrameCodec>>,
           handle: Rc<Handle>)
           -> ConnectedConnection {
        ConnectedConnection {
            cnxn: cnxn,
            tunnel: tunnel,
            handle: handle,
        }
    }

    fn tunnel(&self) -> Result<(), io::Error> {
        self.handle.spawn(self.tunnel.and_then(|connect_frame| {
            if let PunnelFrame::Connect { id, seq } = connect_frame {
                Ok(())
            } else {
                return Err(Error::new(ErrorKind::InvalidInput, "Expected connect"));
            }
        }));
        Ok(())
    }
}

fn get_inaddr_any() -> Ipv6Addr {
    Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 0)
}

fn do_client(local_port: SocketAddr, server_host_port: SocketAddr) {}

fn do_server(connect_to_port: SocketAddr) {
    let mut core = Core::new().unwrap();
    let handle = Rc::new(core.handle());
    let mut local_port: SocketAddr = SocketAddr::V6(SocketAddrV6::new(get_inaddr_any(), 0, 0, 0));
    let acceptor = TcpListener::bind(&local_port, &handle)
        .and_then(move |listener| {
            Ok(listener.incoming().for_each(move |(cnxn, addr)| {
                println!("got connection from {}", addr);
                let (sink, stream) = cnxn.framed(PunnelFrameCodec).split();
                // let t = TunnelConnection::new(Box::new(sink), Box::new(stream), handle.clone());
                // t.connect(connect_to_port);
                Ok(())
            }))
        })
        .unwrap();
    core.run(acceptor).unwrap();
}

fn main() {
    let matches = App::new("persistent tunnel one local port to remote port")
        .version("1.0")
        .subcommand(SubCommand::with_name("client")
            .about("start client and connect to server")
            .arg(Arg::with_name("localPort")
                .help("local port to listen on")
                .index(1)
                .required(true))
            .arg(Arg::with_name("serverHostPort")
                .help("host:port to connect to")
                .index(2)
                .required(true)))
        .subcommand(SubCommand::with_name("server")
            .about("start server bridging to port")
            .arg(Arg::with_name("connectToPort")
                .help("port to connect to")
                .index(1)
                .required(true)))
        .get_matches();

    if let Some(sub_matches) = matches.subcommand_matches("client") {
        let local_port = matches.value_of("localPort").unwrap().parse().unwrap();
        let server_host_port = matches.value_of("serverHostPort").unwrap().parse().unwrap();
        do_client(local_port, server_host_port);
    } else if let Some(sub_matches) = matches.subcommand_matches("server") {
        let connect_to_port = matches.value_of("connectToPort").unwrap().parse().unwrap();
        do_server(connect_to_port);
    }
}
