use std::{
    io::{BufRead, Write},
    net::{Ipv4Addr, SocketAddrV4},
};

fn main() {
    let saddr = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8888);
    println!("listening on address: {}", saddr);
    let listener = std::net::TcpListener::bind(saddr).unwrap();
    for mut stream in listener.incoming().flatten() {
        let mut rdr = std::io::BufReader::new(&mut stream);
        let mut l = String::new();
        rdr.read_line(&mut l).unwrap();
        match l.trim().split(' ').collect::<Vec<_>>().as_slice() {
            ["GET", resource, "HTTP/1.1"] => {
                loop {
                    let mut l = String::new();
                    rdr.read_line(&mut l).unwrap();
                    if l.trim().is_empty() {
                        break;
                    }
                }
                let mut p = std::path::PathBuf::new();
                p.push("webpages");
                p.push(resource.trim_start_matches("/"));
                if resource.ends_with('/') {
                    p.push("index.html");
                }
                println!("{:?}", p);
                stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n").unwrap();
                stream.write_all(&std::fs::read(p).unwrap()).unwrap();
            }
            _ => todo!(),
        }
    }
}
