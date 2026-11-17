use super::super::{Resolved, Source, spec::Spec};
use super::*;
use std::net::TcpListener;
use std::thread::JoinHandle;

const WAIT: Duration = Duration::from_secs(5);

/// A fake proxy: `script` is handed the first connection.
fn serve(script: impl FnOnce(TcpStream) + Send + 'static) -> (u16, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let thread = std::thread::spawn(move || script(listener.accept().unwrap().0));
    (port, thread)
}

fn proxy(protocol: ProxyProtocol, port: u16, login: Option<(&str, &str)>) -> Resolved {
    let spec = Spec {
        protocol,
        host: "127.0.0.1".to_owned(),
        port,
        username: login.map(|(user, _)| user.to_owned()),
        password: login.map(|(_, password)| password.to_owned()),
    };
    Resolved::new(spec, &[], Source::Manual).unwrap()
}

fn read_exactly(stream: &mut TcpStream, count: usize) -> Vec<u8> {
    let mut bytes = vec![0; count];
    stream.read_exact(&mut bytes).unwrap();
    bytes
}

fn read_head(stream: &mut TcpStream) -> String {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        head.extend(read_exactly(stream, 1));
    }
    String::from_utf8(head).unwrap()
}

#[test]
fn base64_matches_the_reference_vectors() {
    for (plain, encoded) in [
        ("", ""),
        ("f", "Zg=="),
        ("fo", "Zm8="),
        ("foo", "Zm9v"),
        ("foob", "Zm9vYg=="),
        ("user:pass", "dXNlcjpwYXNz"),
    ] {
        assert_eq!(base64(plain.as_bytes()), encoded);
    }
}

#[test]
fn connect_names_the_host_and_signs_in() {
    let (port, thread) = serve(|mut stream| {
        let head = read_head(&mut stream);
        assert!(head.starts_with("CONNECT stream.example.com:443 HTTP/1.1\r\n"));
        assert!(head.contains("Host: stream.example.com:443\r\n"));
        assert!(head.contains("Proxy-Authorization: Basic dXNlcjpwYXNz\r\n"));
        // The tunnel's first bytes ride in the same segment as the reply.
        stream
            .write_all(b"HTTP/1.1 200 Connection established\r\n\r\nhello")
            .unwrap();
        assert_eq!(read_exactly(&mut stream, 4), b"ping");
    });
    let proxy = proxy(ProxyProtocol::Http, port, Some(("user", "pass")));
    let mut tunnel = open(&proxy, "stream.example.com", 443, WAIT).unwrap();
    let mut greeting = [0_u8; 5];
    tunnel.read_exact(&mut greeting).unwrap();
    assert_eq!(&greeting, b"hello");
    tunnel.write_all(b"ping").unwrap();
    thread.join().unwrap();
}

#[test]
fn connect_without_a_login_sends_no_credentials() {
    let (port, thread) = serve(|mut stream| {
        let head = read_head(&mut stream);
        assert!(!head.contains("Proxy-Authorization"));
        stream.write_all(b"HTTP/1.0 200 OK\r\n\r\n").unwrap();
    });
    open(
        &proxy(ProxyProtocol::Http, port, None),
        "a.example",
        443,
        WAIT,
    )
    .unwrap();
    thread.join().unwrap();
}

#[test]
fn a_refusing_http_proxy_is_reported_with_its_reason() {
    let (port, thread) = serve(|mut stream| {
        read_head(&mut stream);
        stream
            .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n")
            .unwrap();
    });
    let error = open(
        &proxy(ProxyProtocol::Http, port, None),
        "a.example",
        443,
        WAIT,
    )
    .unwrap_err();
    assert!(
        matches!(&error, TunnelError::Refused(line) if line == "HTTP/1.1 403 Forbidden"),
        "{error}"
    );
    thread.join().unwrap();

    let (port, thread) = serve(|mut stream| {
        read_head(&mut stream);
        stream
            .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n")
            .unwrap();
    });
    let error = open(
        &proxy(ProxyProtocol::Http, port, None),
        "a.example",
        443,
        WAIT,
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("user name and password"),
        "{error}"
    );
    thread.join().unwrap();
}

#[test]
fn something_that_is_not_http_is_not_taken_for_a_proxy() {
    let (port, thread) = serve(|mut stream| {
        read_head(&mut stream);
        stream.write_all(b"SSH-2.0-OpenSSH\r\n\r\n").unwrap();
    });
    let error = open(
        &proxy(ProxyProtocol::Http, port, None),
        "a.example",
        443,
        WAIT,
    )
    .unwrap_err();
    assert!(matches!(error, TunnelError::Protocol(_)), "{error}");
    thread.join().unwrap();
}

#[test]
fn socks5_asks_for_the_host_by_name() {
    let (port, thread) = serve(|mut stream| {
        assert_eq!(read_exactly(&mut stream, 3), [5, 1, 0]);
        stream.write_all(&[5, 0]).unwrap();
        let name = b"stream.example.com";
        let mut expected = vec![5, 1, 0, 3, name.len() as u8];
        expected.extend_from_slice(name);
        expected.extend_from_slice(&443_u16.to_be_bytes());
        assert_eq!(read_exactly(&mut stream, expected.len()), expected);
        stream
            .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0, b'o', b'k'])
            .unwrap();
    });
    let mut tunnel = open(
        &proxy(ProxyProtocol::Socks5h, port, None),
        "stream.example.com",
        443,
        WAIT,
    )
    .unwrap();
    let mut rest = [0_u8; 2];
    tunnel.read_exact(&mut rest).unwrap();
    assert_eq!(&rest, b"ok");
    thread.join().unwrap();
}

#[test]
fn socks5_sends_an_address_as_an_address() {
    let (port, thread) = serve(|mut stream| {
        read_exactly(&mut stream, 3);
        stream.write_all(&[5, 0]).unwrap();
        assert_eq!(
            read_exactly(&mut stream, 10),
            [5, 1, 0, 1, 93, 184, 216, 34, 0x01, 0xBB]
        );
        stream.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).unwrap();
    });
    open(
        &proxy(ProxyProtocol::Socks5, port, None),
        "93.184.216.34",
        443,
        WAIT,
    )
    .unwrap();
    thread.join().unwrap();
}

#[test]
fn socks5_signs_in_with_a_user_name_and_password() {
    let (port, thread) = serve(|mut stream| {
        assert_eq!(read_exactly(&mut stream, 4), [5, 2, 0, 2]);
        stream.write_all(&[5, 2]).unwrap();
        let mut expected = vec![1, 4];
        expected.extend_from_slice(b"user");
        expected.push(4);
        expected.extend_from_slice(b"pass");
        assert_eq!(read_exactly(&mut stream, expected.len()), expected);
        stream.write_all(&[1, 0]).unwrap();
        let head = read_exactly(&mut stream, 5);
        assert_eq!(&head[..4], [5, 1, 0, 3]);
        read_exactly(&mut stream, usize::from(head[4]) + 2);
        stream.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).unwrap();
    });
    let proxy = proxy(ProxyProtocol::Socks5h, port, Some(("user", "pass")));
    open(&proxy, "a.example", 443, WAIT).unwrap();
    thread.join().unwrap();
}

#[test]
fn socks5_reports_a_wrong_password_and_a_failed_connect() {
    let (port, thread) = serve(|mut stream| {
        read_exactly(&mut stream, 4);
        stream.write_all(&[5, 2]).unwrap();
        read_exactly(&mut stream, 11);
        stream.write_all(&[1, 1]).unwrap();
    });
    let error = open(
        &proxy(ProxyProtocol::Socks5h, port, Some(("user", "pass"))),
        "a.example",
        443,
        WAIT,
    )
    .unwrap_err();
    assert!(error.to_string().contains("not accepted"), "{error}");
    thread.join().unwrap();

    let (port, thread) = serve(|mut stream| {
        read_exactly(&mut stream, 3);
        stream.write_all(&[5, 0]).unwrap();
        let head = read_exactly(&mut stream, 5);
        read_exactly(&mut stream, usize::from(head[4]) + 2);
        stream.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]).unwrap();
    });
    let error = open(
        &proxy(ProxyProtocol::Socks5h, port, None),
        "a.example",
        443,
        WAIT,
    )
    .unwrap_err();
    assert!(error.to_string().contains("host refused"), "{error}");
    thread.join().unwrap();
}

#[test]
fn a_socks5_server_that_wants_a_login_we_lack_is_refused() {
    let (port, thread) = serve(|mut stream| {
        read_exactly(&mut stream, 3);
        stream.write_all(&[5, 0xFF]).unwrap();
    });
    let error = open(
        &proxy(ProxyProtocol::Socks5h, port, None),
        "a.example",
        443,
        WAIT,
    )
    .unwrap_err();
    assert!(matches!(error, TunnelError::Refused(_)), "{error}");
    thread.join().unwrap();
}

#[test]
fn proxies_the_tunnel_cannot_speak_are_named() {
    for (protocol, name) in [
        (ProxyProtocol::Https, "HTTPS"),
        (ProxyProtocol::Socks4, "SOCKS4"),
    ] {
        let error = open(&proxy(protocol, 1, None), "a.example", 443, WAIT).unwrap_err();
        assert!(error.to_string().contains(name), "{error}");
    }
}

#[test]
fn a_proxy_that_is_down_is_an_error_not_a_hang() {
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let error = open(
        &proxy(ProxyProtocol::Http, port, None),
        "a.example",
        443,
        WAIT,
    )
    .unwrap_err();
    assert!(matches!(error, TunnelError::Io(_)), "{error}");
}
