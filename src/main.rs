extern crate byteorder;
extern crate clap;
extern crate futures;
extern crate tokio_core;

use clap::{Arg, App, SubCommand};
use futures::{Future, Sink};
use futures::stream::Stream;
use std::io::{self, Error, ErrorKind};
use std::collections::{HashMap, VecDeque};
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6, Shutdown};
use std::sync::{Arc};
use tokio_core::io::{Io, Codec, Framed};
use tokio_core::net::{TcpListener, TcpStream};
use tokio_core::reactor::{Core, Handle};
use std::rc::Rc;

mod punnel_proto;
use punnel_proto::{PunnelFrame, PunnelFrameCodec};

type BoxedPunnelFrameSink = Box<Sink<SinkItem = PunnelFrame, SinkError = io::Error>>;
type BoxedPunnelFrameStream = Box<Stream<Item = PunnelFrame, Error = io::Error>>;

struct ToBeAcked {
    seq: u32,
    buffer: Vec<u8>,
}

struct Connection {
    id: u32,
    stream: Rc<TcpStream>,
    last_ack: u32,
    waiting_for_ack: VecDeque<ToBeAcked>,
}

struct ConnectionRegistry {
    connections: HashMap<u32, Arc<Connection>>,
    next_key: u32,
}

impl ConnectionRegistry {
    // for use by the server to add a connection
    fn register(&mut self, tcp_stream: TcpStream) -> Arc<Connection> {
        // id 0 is special (indicating no connection, so make sure to skip it
        while self.next_key != 0 && self.connections.contains_key(&self.next_key) {
            self.next_key += 1
        }

        let key = self.next_key;
        self.next_key += 1;
        self.add(key, tcp_stream)
    }

    // for use by the client to add a connection
    fn add(&mut self, id: u32, tcp_stream: TcpStream) -> Arc<Connection> {
        let cnxn = Connection {
            id: id,
            stream: Rc::new(tcp_stream),
            last_ack: 0,
            waiting_for_ack: VecDeque::new(),
        };

        let rc_cnxn = Arc::new(cnxn);
        self.connections.insert(id, rc_cnxn.clone());
        rc_cnxn
    }

    fn get_connection(&self, id: u32) -> Option<&Arc<Connection>> {
        self.connections.get(&id)
    }

    fn close(&mut self, id: u32) {
        if let Some(cnxn) = self.connections.remove(&id) {
            // todo close connection
        }
    }
}

struct ConnectedConnection {
    registry: Rc<ConnectionRegistry>,
    tunnel_sink: BoxedPunnelFrameSink,
    tunnel_stream: BoxedPunnelFrameStream,
    handle: Arc<Handle>,
}

impl ConnectedConnection {
    fn new(registry: Rc<ConnectionRegistry>,
           tunnel_sink: BoxedPunnelFrameSink,
           tunnel_stream: BoxedPunnelFrameStream,
           handle: Arc<Handle>)
           -> ConnectedConnection {
        ConnectedConnection {
            registry: registry,
            tunnel_sink: tunnel_sink,
            tunnel_stream: tunnel_stream,
            handle: handle,
        }
    }

    fn tunnel(self) -> Result<(), io::Error> {
        let registry = &self.registry;

        let stream = self.tunnel_stream.and_then(|connect_frame| {
            if let PunnelFrame::Connect { id, seq } = connect_frame {
                if id == 0 {
                    // create a new connection
                    //Ok(create_new_connection())
                    Err(Error::new(ErrorKind::InvalidInput, "Expected connect"))
                } else {
                    // find an existing connection
                    match registry.get_connection(id) {
                        Some(cnxn) => Ok((cnxn, seq)),
                        None => Err(Error::new(ErrorKind::InvalidInput, "invalid id")),
                    }
                }
            } else {
                 Err(Error::new(ErrorKind::InvalidInput, "Expected connect"))
            }
        }).and_then(|(cnxn, seq)| {
            let write_stream = cnxn.clone();
            let read_stream = cnxn.clone();
            Ok(())
        });
        Ok(())
    }
}

fn get_inaddr_any() -> Ipv6Addr {
    Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 0)
}

fn do_client(local_port: SocketAddr, server_host_port: SocketAddr) {}

struct IncomingConnection {
    tunnel_sink: BoxedPunnelFrameSink,
    tunnel_stream: BoxedPunnelFrameStream,
}

impl IncomingConnection {
    fn process(self) {
        self.tunnel_stream.and_then(|p| {
            if let PunnelFrame::Connect{id, seq} = p {
                if id == 0 {
                    // create a new session
                } else {
                    // reconnect
                }
                Ok(())
            } else {
                Err(Error::new(ErrorKind::InvalidInput, "Expected connect"))
            }
        });
    }
}

fn do_server(connect_to_port: SocketAddr) {
    let mut core = Core::new().unwrap();
    let handle = Arc::new(core.handle());
    let mut local_port: SocketAddr = SocketAddr::V6(SocketAddrV6::new(get_inaddr_any(), 0, 0, 0));
    let acceptor = TcpListener::bind(&local_port, &handle)
        .and_then(move |listener| {
            Ok(listener.incoming().for_each(|(cnxn, addr)| {
                let (sink, stream) = cnxn.framed(PunnelFrameCodec).split();
                IncomingConnection {
                    tunnel_sink: Box::new(sink),
                    tunnel_stream: Box::new(stream),
                }.process();
                Ok(())
            }))
        })
        .unwrap();
    core.run(acceptor).unwrap();
}

fn main() {
    let mut app = App::new("persistent tunnel one local port to remote port")
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
                .required(true)));
    let matches = app.clone().get_matches();

    match matches.subcommand() {
        ("client", Some(sub_matches)) => {
            let local_port = matches.value_of("localPort").unwrap().parse().unwrap();
            let server_host_port = matches.value_of("serverHostPort").unwrap().parse().unwrap();
            do_client(local_port, server_host_port);
        },
        ("server", Some(sub_matches)) => {
            let connect_to_port = matches.value_of("connectToPort").unwrap().parse().unwrap();
            do_server(connect_to_port);
        },
        _ => { app.print_help(); }
    }
}
