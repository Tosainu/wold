use std::net::SocketAddr;

use anyhow::{Context, Result};
use axum::{http::StatusCode, response::IntoResponse, routing::post, Json, Router};
use tokio::net::UdpSocket;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    let dst = SocketAddr::from(([255; 4], 9));

    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let args = parse_command_line(&args)?;

    match args {
        CmdLine::Help => help(addr, dst),

        CmdLine::Run {
            listen_addr,
            broadcast_addr,
        } => {
            let addr = listen_addr.unwrap_or(addr);
            let dst = broadcast_addr.unwrap_or(dst);
            run(addr, dst).await
        }
    }
}

fn help(addr: SocketAddr, dst: SocketAddr) -> Result<()> {
    print!(
        "USAGE:
    wold [OPTIONS]

OPTIONS:
    -l <address>:<port>     start a server with a provided address (default: {addr})
    -b <address>:<port>     send magic packets to a provided address (default: {dst})

    --help, -h              display this message and exit
"
    );
    Ok(())
}

async fn run(addr: SocketAddr, dst: SocketAddr) -> Result<()> {
    tracing::debug!("listening on {addr}");
    tracing::debug!("wol dst addr: {dst}");

    let app = Router::new().route("/", post(move |req| handle_wol_request(dst, req)));

    axum::Server::try_bind(&addr)
        .context("failed to start server")?
        .serve(app.into_make_service())
        .await
        .context("server error")?;

    Ok(())
}

#[derive(Debug, PartialEq)]
enum CmdLine {
    Help,
    Run {
        listen_addr: Option<SocketAddr>,
        broadcast_addr: Option<SocketAddr>,
    },
}

fn parse_command_line<T: AsRef<str>>(args: &[T]) -> Result<CmdLine> {
    let mut listen_addr = None;
    let mut broadcast_addr = None;

    let mut args = args.iter().peekable();
    while let (Some(opt), value) = (args.next().map(AsRef::as_ref), args.peek()) {
        match (opt, value) {
            ("--help" | "-h", _) => return Ok(CmdLine::Help),

            ("-l", Some(_)) => {
                let value = args.next().unwrap().as_ref();
                listen_addr.replace(value.parse().with_context(|| {
                    format!("failed to parse command line: '{opt}', '{value}'")
                })?);
            }
            ("-d", Some(_)) => {
                let value = args.next().unwrap().as_ref();
                broadcast_addr.replace(value.parse().with_context(|| {
                    format!("failed to parse command line: '{opt}', '{value}'")
                })?);
            }

            _ => return Err(anyhow::anyhow!("unknown option: {opt}")),
        }
    }

    Ok(CmdLine::Run {
        listen_addr,
        broadcast_addr,
    })
}

#[derive(Debug, serde::Deserialize)]
struct Req {
    #[serde(with = "serde_bytes")]
    target: Vec<u8>,
}

fn eui48(s: &[u8]) -> Option<[u8; 6]> {
    if s.len() != 2 * 6 + 5 {
        return None;
    }

    fn f(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'A'..=b'F' => Some(c - b'A' + 0xa),
            b'a'..=b'f' => Some(c - b'a' + 0xa),
            _ => None,
        }
    }

    let mut mac = [0; 6];
    for (i, m) in mac.iter_mut().enumerate() {
        if let Some([c1, c2, b':' | b'-', ..] | [c1, c2]) = s.get(i * 3..) {
            if let (Some(a), Some(b)) = (f(*c1), f(*c2)) {
                *m = a.wrapping_shl(4) | b;
                continue;
            }
        }

        return None;
    }

    Some(mac)
}

#[cfg(test)]
mod test {
    #[test]
    fn parse_command_line() {
        use super::{parse_command_line, CmdLine};
        use std::net::SocketAddr;

        assert_eq!(
            parse_command_line(&[] as &[&str]).unwrap(),
            CmdLine::Run {
                listen_addr: None,
                broadcast_addr: None
            }
        );

        assert_eq!(
            parse_command_line(&["-l", "127.0.0.1:3000"]).unwrap(),
            CmdLine::Run {
                listen_addr: Some(SocketAddr::from(([127, 0, 0, 1], 3000))),
                broadcast_addr: None
            }
        );
        assert_eq!(
            parse_command_line(&["-d", "127.0.0.1:3000"]).unwrap(),
            CmdLine::Run {
                listen_addr: None,
                broadcast_addr: Some(SocketAddr::from(([127, 0, 0, 1], 3000))),
            }
        );

        assert_eq!(parse_command_line(&["--help"]).unwrap(), CmdLine::Help);
        assert_eq!(parse_command_line(&["-h"]).unwrap(), CmdLine::Help);
        assert_eq!(
            parse_command_line(&["-h", "-l", "127.0.0.1:3000"]).unwrap(),
            CmdLine::Help
        );
        assert_eq!(
            parse_command_line(&["-l", "127.0.0.1:3000", "-h"]).unwrap(),
            CmdLine::Help
        );
    }

    #[test]
    fn eui48() {
        use super::eui48;

        assert_eq!(
            eui48(b"01:23:45:67:89:ab"),
            Some([0x01, 0x23, 0x45, 0x67, 0x89, 0xab])
        );
        assert_eq!(
            eui48(b"01-23-45-67-89-ab"),
            Some([0x01, 0x23, 0x45, 0x67, 0x89, 0xab])
        );
        assert_eq!(
            eui48(b"01:23-45:67-89:ab"),
            Some([0x01, 0x23, 0x45, 0x67, 0x89, 0xab])
        );
        assert_eq!(eui48(b"01:23:45:67:89"), None);
        assert_eq!(eui48(b"001:23:45:67:89:ab"), None);
    }
}

async fn handle_wol_request(dst: SocketAddr, Json(req): Json<Req>) -> impl IntoResponse {
    tracing::debug!("got: {req:?}");

    match eui48(&req.target) {
        Some(target) => match wol(dst, target).await {
            Ok(_) => StatusCode::OK,
            Err(err) => {
                tracing::warn!("failed to send magic packet: {err}");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        },
        None => StatusCode::BAD_REQUEST,
    }
}

async fn wol(dst: SocketAddr, mac_addr: [u8; 6]) -> std::io::Result<()> {
    let magic = unsafe {
        let mut a = std::mem::MaybeUninit::<[u8; 102]>::uninit();
        let p = a.as_mut_ptr();
        (*p)[0..6].copy_from_slice(&MAGIC_PACKET_HEADER);
        for pp in (*p)[6..].chunks_mut(6) {
            pp.copy_from_slice(&mac_addr);
        }
        a.assume_init()
    };

    let sock = UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0))).await?;
    sock.set_broadcast(true)?;
    sock.send_to(&magic, dst).await?;

    Ok(())
}

const MAGIC_PACKET_HEADER: [u8; 6] = [0xffu8; 6];
