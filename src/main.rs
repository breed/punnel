extern crate byteorder;
extern crate clap;
extern crate futures;
extern crate tokio_core;

use clap::{Arg, App, SubCommand};
use futures::{Future, Sink};
use futures::stream::{Stream};
use std::io::{self};
use std::collections::{HashMap};
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6, Shutdown};
use tokio_core::io::{Io, Codec, Framed};
use tokio_core::net::{TcpListener, TcpStream};
use tokio_core::reactor::{Core, Handle};
use std::rc::Rc;

mod punnel_proto;
use punnel_proto::{PunnelFrame, PunnelFrameCodec};

type BoxedPunnelFrameSink = Box<Sink<SinkItem=PunnelFrame, SinkError=io::Error>>;
type BoxedPunnelFrameStream = Box<Stream<Item=PunnelFrame, Error=io::Error>>;

struct Connection<C: Codec + 'static>  {
    id: u32,
    framed: Rc<Framed<TcpStream, C>>,
}

struct TunnelConnection {
    sink1: BoxedPunnelFrameSink,
    stream1: BoxedPunnelFrameStream,
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
    fn new(sink1: BoxedPunnelFrameSink,
           stream1: BoxedPunnelFrameStream,
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
            let (sink2, stream2) = cnxn.framed(PunnelFrameCodec).split();
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

fn get_inaddr_any() -> Ipv6Addr {
    Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 0)
}

fn do_client(local_port: SocketAddr, server_host_port: SocketAddr) {
}

fn do_server(connect_to_port: SocketAddr) {
    let mut core = Core::new().unwrap();
    let handle = Rc::new(core.handle());
    let mut local_port: SocketAddr = SocketAddr::V6(SocketAddrV6::new(get_inaddr_any(), 0, 0, 0));
    let acceptor = TcpListener::bind(&local_port, &handle).and_then(move |listener| {
        Ok(listener.incoming().for_each(move |(cnxn, addr)| {
                println!("got connection from {}", addr);
                let (sink, stream) = cnxn.framed(PunnelFrameCodec).split();
                let t = TunnelConnection::new(Box::new(sink), Box::new(stream), handle.clone());
                t.connect(connect_to_port);
                Ok(())
        }))
    }).unwrap();
    core.run(acceptor).unwrap();
}

fn main() {
    let matches = App::new("persistent tunnel one local port to remote port")
            .version("1.0")
            .subcommand(
                SubCommand::with_name("client")
                    .about("start client and connect to server")
                    .arg(Arg::with_name("localPort")
                         .help("local port to listen on")
                         .index(1)
                         .required(true))
                    .arg(Arg::with_name("serverHostPort")
                         .help("host:port to connect to")
                         .index(2)
                         .required(true)))
            .subcommand(
                SubCommand::with_name("server")
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
