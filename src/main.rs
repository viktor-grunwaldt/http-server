use std::{
    borrow::Cow,
    env::{self, Args},
    fs,
    io::{BufRead, Write},
    net::{Ipv4Addr, SocketAddrV4, TcpStream},
    path::{Path, PathBuf},
};

enum Status {
    Success,
    MovedPermamently(String),
    Forbidden,
    PageNotFound,
    InternalServerError,
}
const NOT_FOUND_BODY: &str = "File not found or unsupported type.";
const CONTENT_TYPE_TEXT: &str = "text/plain; charset=utf-8";

// as I'm using the format! macro, the format literal needs to be known at compile time
// https://github.com/rust-lang/rust/issues/69133
macro_rules! HTML_MOVED {() => (
    "<!DOCTYPE html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>{}</title></head>\n<body>\n<h1>{}</h1>\n<p>The document has moved <a href=\"{}\">here</a>.</p>\n</body>\n</html>"
)}

fn build_http_response(
    status: Status,
    content_type: &str,
    body: Cow<'static, [u8]>,
) -> Cow<'static, [u8]> {
    #[rustfmt::skip]
    let (code, status_str) = match status {
        Status::Success             => (200, "OK"),
        Status::MovedPermamently(_) => (301, "Moved Permamently"),
        Status::Forbidden           => (403, "Forbidden"),
        Status::PageNotFound        => (404, "Not Found"),
        Status::InternalServerError => (500, "Internal Server Error"),
    };

    let full_status_line = format!("HTTP/1.1 {} {}", code, status_str);
    let len = body.len();

    let mut headers = format!(
        "Content-Type: {}\r\nContent-Length: {}\r\n\r\n",
        content_type, len
    );
    let is_body_empty = body.is_empty();
    let mut final_body = body;
    if let Status::MovedPermamently(url) = status {
        if is_body_empty {
            let html = format!(HTML_MOVED!(), status_str, status_str, &url);
            final_body = Cow::Owned(html.into_bytes());
        }
        headers.push_str(&format!("Location: {}\r\n", url));
    }

    let mut response_bytes =
        Vec::with_capacity(full_status_line.len() + 2 + headers.len() + final_body.len());
    response_bytes.extend_from_slice(full_status_line.as_bytes());
    response_bytes.extend_from_slice(b"\r\n"); // CRLF after status line
    response_bytes.extend_from_slice(headers.as_bytes());
    response_bytes.extend_from_slice(&final_body);

    Cow::Owned(response_bytes)
}

fn e_to_cow(p: &Path, e: std::io::Error) -> Cow<'static, [u8]> {
    eprintln!("Error reading file {}: {}", p.display(), e);
    let body = format!("Server error: {}", e);
    build_http_response(
        Status::InternalServerError,
        CONTENT_TYPE_TEXT,
        Cow::Owned(body.into_bytes()),
    )
}

fn build_response_other(ext: &str, p: &Path) -> Cow<'static, [u8]> {
    // Attempt to guess the Content-Type based on the extension
    let content_type = match ext.to_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "xml" => "application/xml",
        "css" => "text/css",
        "js" => "application/javascript",
        "txt" => CONTENT_TYPE_TEXT,
        "bin" => "application/octet-stream", // Generic binary
        _ => "application/octet-stream",     // Default for unknown
    };

    match fs::read(p) {
        // Read file as bytes
        Ok(file_bytes) => {
            build_http_response(Status::Success, content_type, Cow::Owned(file_bytes))
        }
        Err(e) => e_to_cow(p, e),
    }
}
fn handle_request(mut p: PathBuf, resource: &str, url: String) -> Cow<'static, [u8]> {
    if p.is_dir() {
        let spare_slash = if resource.ends_with('/') { "" } else { "/" };
        let redirect_url = format!("{}{}{}index.html", url, resource, spare_slash);
        #[cfg(debug_assertions)]
        println!("{}", redirect_url);
        return build_http_response(
            Status::MovedPermamently(redirect_url),
            CONTENT_TYPE_TEXT,
            Cow::Owned(vec![]),
        );
    }
    p.push(resource.trim_start_matches("/"));
    match p.extension().and_then(|ext| ext.to_str()) {
        Some("html") => match fs::read_to_string(&p) {
            Ok(file_content) => build_http_response(
                Status::Success,
                "text/html; charset=utf-8",
                Cow::Owned(file_content.into_bytes()),
            ),
            Err(e) => e_to_cow(&p, e),
        },
        Some(ext) => build_response_other(ext, &p),
        _ => {
            eprintln!("Unhandled path or file extension: {}", p.display());
            build_http_response(
                Status::PageNotFound,
                CONTENT_TYPE_TEXT,
                Cow::Borrowed(NOT_FOUND_BODY.as_bytes()),
            )
        }
    }
}

fn parse_host_address(host_str: &str) -> Option<&str> {
    host_str
        .strip_prefix("Host: ")
        .and_then(|x| x.strip_prefix("http://").or(Some(x)))
        .and_then(|x| x.split('/').next())
        .map(|x| x.split_once(':').map_or(x, |(name, _port)| name))
}

fn handle_connection(resource_dir: &Path, mut stream: TcpStream, addr: SocketAddrV4) {
    let mut rdr = std::io::BufReader::new(&mut stream);
    let mut l = String::new();
    rdr.read_line(&mut l).unwrap();
    match l.trim().split(' ').collect::<Vec<_>>().as_slice() {
        ["GET", resource, "HTTP/1.1"] => {
            let remainder = rdr
                .lines()
                .take_while(|x| x.as_ref().map(|l| !l.is_empty()).unwrap_or(true))
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            #[cfg(debug_assertions)]
            println!("{:?}", remainder);
            let domain_name = remainder
                .get(0)
                .and_then(|x| parse_host_address(x.as_str()))
                .expect("couldn't parse or find the Host header");
            let mut p = std::path::PathBuf::new();
            p.push(&resource_dir);
            if env::var("HOST_NOT_DEFINED").unwrap_or_default() != "1" {
                p.push(domain_name);
            }
            let url = format!("http://{domain_name}:{}", addr.port());
            let response = handle_request(p, resource, url);
            stream.write_all(&response).unwrap();
        }
        _ => todo!(),
    }
}
struct ProgArgs {
    port: u16,
    directory: PathBuf,
}
fn parse_args(mut args: Args) -> Option<ProgArgs> {
    let _name = args.next()?;
    let port = args.next()?.parse().ok()?;
    let directory = args.next()?.parse().ok()?;
    Some(ProgArgs { port, directory })
}
fn main() {
    let args = match parse_args(env::args()) {
        Some(x) => x,
        None => {
            eprintln!("usage: http_server [port] [directory]");
            std::process::exit(1);
        }
    };

    let saddr = SocketAddrV4::new(Ipv4Addr::new(127, 0, 1, 1), args.port);
    println!("listening on address: http://{}", saddr);
    let listener = std::net::TcpListener::bind(saddr).unwrap();
    listener
        .incoming()
        .flatten()
        .for_each(|s| handle_connection(&args.directory, s, saddr));
}
