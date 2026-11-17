//! A TCP connection to a host through the proxy, for the WebSocket live feeds (`ureq` does this
//! for HTTP on its own). The tunnel is opened first and TLS is then spoken over it, so the proxy
//! only ever sees the host and port. HTTP proxies are asked with `CONNECT`; SOCKS5 proxies with
//! or without a user name and password. SOCKS4 and HTTPS proxies are not spoken here.

use super::{Resolved, refresh_if_stale, route};
use crate::model::encoding::base64_encode;
use std::io::{self, Read, Write};
use std::net::{IpAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;
use ureq::ProxyProtocol;

/// The head of an HTTP reply is read up to here; a proxy that sends more is not one.
const MAX_HEAD: usize = 8 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum TunnelError {
    #[error("Live feeds cannot go through {0} proxies; use an HTTP or SOCKS5 proxy")]
    Unsupported(&'static str),
    /// A direct connection (no proxy for this host) failed.
    #[error("{0}")]
    Connect(io::Error),
    /// Reaching or talking to the proxy failed.
    #[error("proxy: {0}")]
    Proxy(#[from] io::Error),
    #[error("The proxy refused the connection: {0}")]
    Refused(String),
    #[error("The proxy did not answer as expected: {0}")]
    Protocol(&'static str),
}

/// A connection to `host:port`: through the proxy in force unless it skips this host. Blocks; it
/// may refresh the system's proxy first.
pub fn dial(host: &str, port: u16, timeout: Duration) -> Result<TcpStream, TunnelError> {
    refresh_if_stale();
    match route(host) {
        Some(proxy) => open(&proxy, host, port, timeout),
        None => connect(host, port, timeout).map_err(TunnelError::Connect),
    }
}

/// A tunnel to `host:port` through `proxy`.
pub fn open(
    proxy: &Resolved,
    host: &str,
    port: u16,
    timeout: Duration,
) -> Result<TcpStream, TunnelError> {
    let handshake = match proxy.spec.protocol {
        ProxyProtocol::Http => http_connect,
        ProxyProtocol::Socks5 | ProxyProtocol::Socks5h => socks5,
        ProxyProtocol::Https => return Err(TunnelError::Unsupported("HTTPS")),
        _ => return Err(TunnelError::Unsupported("SOCKS4")),
    };
    let mut stream = connect(&proxy.spec.host, proxy.spec.port, timeout)?;
    let credentials = proxy
        .spec
        .username
        .as_deref()
        .map(|user| (user, proxy.spec.password.as_deref().unwrap_or_default()));
    handshake(&mut stream, host, port, credentials)?;
    Ok(stream)
}

/// A plain TCP connection with `timeout` for the connect and for each read and write after.
fn connect(host: &str, port: u16, timeout: Duration) -> io::Result<TcpStream> {
    let mut last = None;
    // A URL's IPv6 host has brackets, a socket address does not.
    let host = super::spec::bare_host(host);
    for address in (host, port).to_socket_addrs()?.take(4) {
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(stream) => {
                stream.set_read_timeout(Some(timeout))?;
                stream.set_write_timeout(Some(timeout))?;
                return Ok(stream);
            }
            Err(error) => last = Some(error),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("no address for host")))
}

fn http_connect(
    stream: &mut TcpStream,
    host: &str,
    port: u16,
    credentials: Option<(&str, &str)>,
) -> Result<(), TunnelError> {
    let authority = format!("{}:{port}", super::spec::bracketed(host));
    let mut request = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n");
    if let Some((user, password)) = credentials {
        let token = base64_encode(format!("{user}:{password}").as_bytes());
        request.push_str(&format!("Proxy-Authorization: Basic {token}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes())?;

    // One byte at a time: anything after the blank line already belongs to the tunnel.
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() >= MAX_HEAD {
            return Err(TunnelError::Protocol("reply too long"));
        }
        if stream.read(&mut byte)? == 0 {
            return Err(TunnelError::Protocol("connection closed"));
        }
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head);
    let line = head.lines().next().unwrap_or_default();
    let mut parts = line.splitn(3, ' ');
    let (version, status) = (
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
    );
    if !version.starts_with("HTTP/") {
        return Err(TunnelError::Protocol("not an HTTP reply"));
    }
    match status {
        "200" => Ok(()),
        "407" => Err(TunnelError::Refused(
            "it needs a user name and password".to_owned(),
        )),
        _ => Err(TunnelError::Refused(line.to_owned())),
    }
}

fn socks5(
    stream: &mut TcpStream,
    host: &str,
    port: u16,
    credentials: Option<(&str, &str)>,
) -> Result<(), TunnelError> {
    const NO_AUTH: u8 = 0;
    const PASSWORD: u8 = 2;
    let greeting: &[u8] = if credentials.is_some() {
        &[5, 2, NO_AUTH, PASSWORD]
    } else {
        &[5, 1, NO_AUTH]
    };
    stream.write_all(greeting)?;
    let mut choice = [0_u8; 2];
    stream.read_exact(&mut choice)?;
    if choice[0] != 5 {
        return Err(TunnelError::Protocol("not a SOCKS5 server"));
    }
    match (choice[1], credentials) {
        (NO_AUTH, _) => {}
        (PASSWORD, Some((user, password))) => {
            let (user, password) = (user.as_bytes(), password.as_bytes());
            let (Ok(user_len), Ok(password_len)) =
                (u8::try_from(user.len()), u8::try_from(password.len()))
            else {
                return Err(TunnelError::Protocol("user name or password too long"));
            };
            let mut message = vec![1, user_len];
            message.extend_from_slice(user);
            message.push(password_len);
            message.extend_from_slice(password);
            stream.write_all(&message)?;
            // RFC 1929: the sub-negotiation's version is 1, and status 0 is success.
            let mut verdict = [0_u8; 2];
            stream.read_exact(&mut verdict)?;
            if verdict[0] != 1 {
                return Err(TunnelError::Protocol("unexpected sign-in reply"));
            }
            if verdict[1] != 0 {
                return Err(TunnelError::Refused(
                    "the user name or password was not accepted".to_owned(),
                ));
            }
        }
        (0xFF, _) => {
            return Err(TunnelError::Refused(
                "it accepts none of the ways to sign in offered".to_owned(),
            ));
        }
        _ => return Err(TunnelError::Protocol("unexpected sign-in method")),
    }

    let mut request = vec![5, 1, 0];
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) => {
            request.push(1);
            request.extend_from_slice(&address.octets());
        }
        Ok(IpAddr::V6(address)) => {
            request.push(4);
            request.extend_from_slice(&address.octets());
        }
        Err(_) => {
            let name = host.as_bytes();
            let Ok(length) = u8::try_from(name.len()) else {
                return Err(TunnelError::Protocol("host name too long"));
            };
            request.extend_from_slice(&[3, length]);
            request.extend_from_slice(name);
        }
    }
    request.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&request)?;

    let mut reply = [0_u8; 4];
    stream.read_exact(&mut reply)?;
    if reply[0] != 5 {
        return Err(TunnelError::Protocol("not a SOCKS5 server"));
    }
    if reply[1] != 0 {
        return Err(TunnelError::Refused(socks5_failure(reply[1]).to_owned()));
    }
    let address_len = match reply[3] {
        1 => 4,
        4 => 16,
        3 => {
            let mut length = [0_u8; 1];
            stream.read_exact(&mut length)?;
            usize::from(length[0])
        }
        _ => return Err(TunnelError::Protocol("unknown address type")),
    };
    let mut rest = vec![0_u8; address_len + 2];
    stream.read_exact(&mut rest)?;
    Ok(())
}

fn socks5_failure(code: u8) -> &'static str {
    match code {
        2 => "the rules of the proxy do not allow it",
        3 => "the network is unreachable",
        4 => "the host is unreachable",
        5 => "the host refused the connection",
        6 => "the connection timed out",
        7 => "command not supported",
        8 => "address type not supported",
        _ => "general failure",
    }
}

#[cfg(test)]
mod tests;
