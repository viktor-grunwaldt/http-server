use std::{
    borrow::Cow,
    env::{self, Args},
    fs,
    io::{BufRead, Write},
    net::{Ipv4Addr, SocketAddrV4, TcpStream},
    path::{Path, PathBuf},
};

enum Status {
    InternalServerError,
    PageNotFound,
    Forbidden,
    Success,
}

fn build_http_response(
    status: Status,
    content_type: &str,
    body: Cow<'static, [u8]>,
) -> Cow<'static, [u8]> {
    let (status_line_str, _) = match status {
        Status::Success => ("200 OK", "OK"),
        Status::PageNotFound => ("404 Not Found", "Not Found"),
        Status::Forbidden => ("403 Forbidden", "Forbidden"),
        Status::InternalServerError => ("500 Internal Server Error", "Internal Server Error"),
    };

    let full_status_line = format!("HTTP/1.1 {}", status_line_str);
    let len = body.len();

    let headers = format!(
        "Content-Type: {}\r\nContent-Length: {}\r\n\r\n",
        content_type, len
    );

    let mut response_bytes =
        Vec::with_capacity(full_status_line.len() + 2 + headers.len() + body.len());
    response_bytes.extend_from_slice(full_status_line.as_bytes());
    response_bytes.extend_from_slice(b"\r\n"); // CRLF after status line
    response_bytes.extend_from_slice(headers.as_bytes());
    response_bytes.extend_from_slice(&body);

    Cow::Owned(response_bytes)
}

fn e_to_cow(p: &Path, e: std::io::Error) -> Cow<'static, [u8]> {
    eprintln!("Error reading file {}: {}", p.display(), e);
    let body = format!("Server error: {}", e);
    build_http_response(
        Status::InternalServerError,
        "text/plain; charset=utf-8",
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
        "txt" => "text/plain; charset=utf-8",
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

fn handle_request(p: &Path) -> Cow<'static, [u8]> {
    match p.extension().and_then(|ext| ext.to_str()) {
        Some("html") => match fs::read_to_string(p) {
            Ok(file_content) => build_http_response(
                Status::Success,
                "text/html; charset=utf-8",
                Cow::Owned(file_content.into_bytes()),
            ),
            Err(e) => e_to_cow(p, e),
        },
        Some(ext) => build_response_other(ext, p),
        _ => {
            eprintln!("Unhandled path or file extension: {}", p.display());
            let body = "File not found or unsupported type.";
            build_http_response(
                Status::PageNotFound,
                "text/plain; charset=utf-8",
                Cow::Borrowed(body.as_bytes()),
            )
        }
    }
}

fn handle_connection(files: &Path, mut stream: TcpStream) {
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
            let mut p = std::path::PathBuf::new();
            p.push(&files);
            p.push(resource.trim_start_matches("/"));
            if resource.ends_with('/') {
                p.push("index.html");
            }
            println!("{:?}", p);
            let response = handle_request(&p);
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
        .for_each(|s| handle_connection(&args.directory, s));
}
