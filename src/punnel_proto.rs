extern crate byteorder;
extern crate tokio_core;

use byteorder::{BigEndian, ByteOrder};
use std::io::{self, Error, ErrorKind};
use tokio_core::io::{Codec, EasyBuf};

const CONNECT: u8 = 1;
const DATA: u8 = 2;
const ACK: u8 = 3;
const CLOSE: u8 = 4;

pub enum PunnelFrame {
    /* an id of 0 means that we are making a new connection,
     * on request the seq is the last packet received from
     * server, on responce is is the last packet received by
     * server. */
    Connect {id: u32, seq: u32},
    Data {seq: u32, data: Vec<u8>,},
    Ack {seq: u32 },
    Close,
}

pub struct PunnelFrameCodec;

fn write_u32(u: u32, into: &mut Vec<u8>) {
    let a: &mut [u8; 4] = &mut [0; 4];
    BigEndian::write_u32(a, u);
    into.extend_from_slice(a);
}

impl Codec for PunnelFrameCodec {
    type In = PunnelFrame;
    type Out = PunnelFrame;

    fn decode(&mut self, buf: &mut EasyBuf) -> Result<Option<PunnelFrame>, io::Error> {
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
                    Ok(Some(PunnelFrame::Connect {
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
                        Ok(Some(PunnelFrame::Data {
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
                    Ok(Some(PunnelFrame::Ack {
                        seq: BigEndian::read_u32(&slice[1..4]),
                    }))
                }
            },
            CLOSE => Ok(Some(PunnelFrame::Close)),
            _ => Err(Error::new(ErrorKind::InvalidInput, "Unknown type")),
        }
    }

    fn encode(&mut self, item: PunnelFrame, into: &mut Vec<u8>) -> io::Result<()> {
        match item {
            PunnelFrame::Connect{id, seq} => {
                into.push(CONNECT);
                write_u32(id, into);
                write_u32(seq, into)
            },
            PunnelFrame::Data{seq, data} => {
                into.push(DATA);
                let data_len = data.len();
                write_u32(seq, into);
                write_u32(data_len as u32, into);
                into.extend(data)
            },
            PunnelFrame::Ack{seq} => {
                into.push(ACK);
                write_u32(seq, into);
            },
            PunnelFrame::Close => {
                into.push(CLOSE)
            },
        }

        Ok(())
    }
}
