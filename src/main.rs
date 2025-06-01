use std::{
    borrow::Cow,
    env::{self, Args},
    fs,
    io::{BufRead, Write},
    net::{Ipv4Addr, SocketAddrV4, TcpStream},
    path::{Path, PathBuf},
};

#[derive(Clone)]
enum Status {
    Success,
    MovedPermamently(String),
    Forbidden,
    PageNotFound,
    InternalServerError,
    NotImplemented,
}
const NOT_FOUND_BODY: &str = "File not found or unsupported type.";

// as I'm using the format! macro, the format literal needs to be known at compile time
// https://github.com/rust-lang/rust/issues/69133
macro_rules! HTML_MOVED {() => (
    "<!DOCTYPE html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>{}</title></head>\n<body>\n<h1>{}</h1>\n<p>The document has moved <a href=\"{}\">here</a>.</p>\n</body>\n</html>"
)}

macro_rules! HTML_ERROR {() => (
    "<!DOCTYPE html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>{0}</title></head>\n<body>\n<h1>{0}</h1>\n</body>\n</html>"
)}

#[rustfmt::skip]
fn from_status(s: Status) -> (u16, &'static str) {
    match s {
        Status::Success             => (200, "OK"),
        Status::MovedPermamently(_) => (301, "Moved Permamently"),
        Status::Forbidden           => (403, "Forbidden"),
        Status::PageNotFound        => (404, "Not Found"),
        Status::InternalServerError => (500, "Internal Server Error"),
        Status::NotImplemented      => (501, "Not Implemented"),
    }
}

fn build_error_response(status: Status) -> Cow<'static, [u8]> {
    let body = format!(HTML_ERROR!(), from_status(status.clone()).1,);
    build_http_response(
        status,
        "text/html; charset=utf-8",
        Cow::Owned(body.into_bytes()),
    )
}
fn build_http_response(
    status: Status,
    content_type: &str,
    initial_body: Cow<'static, [u8]>,
) -> Cow<'static, [u8]> {
    let (code, status_str) = from_status(status.clone());
    let full_status_line = format!("HTTP/1.1 {} {}", code, status_str);

    let mut headers = String::new();
    let mut final_body = initial_body;

    if let Status::MovedPermamently(url) = status {
        if final_body.is_empty() {
            let html = format!(HTML_MOVED!(), status_str, status_str, &url);
            final_body = Cow::Owned(html.into_bytes());
        }
        headers.push_str(&format!("Location: {}\r\n", url));
    }

    headers.push_str(&format!("Content-Type: {}\r\n", content_type));
    headers.push_str(&format!("Content-Length: {}\r\n", final_body.len()));

    let mut response_bytes = vec![];
    response_bytes.extend_from_slice(full_status_line.as_bytes());
    response_bytes.extend_from_slice(b"\r\n");
    response_bytes.extend_from_slice(headers.as_bytes());
    response_bytes.extend_from_slice(b"\r\n");
    response_bytes.extend_from_slice(&final_body);

    Cow::Owned(response_bytes)
}

fn e_to_cow(p: &Path, e: std::io::Error) -> Cow<'static, [u8]> {
    eprintln!("Error reading file {}: {}", p.display(), e);
    build_error_response(Status::InternalServerError)
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
        "bin" => "application/octet-stream",
        _ => "application/octet-stream",
    };

    match fs::read(p) {
        Ok(file_bytes) => {
            build_http_response(Status::Success, content_type, Cow::Owned(file_bytes))
        }
        Err(e) => e_to_cow(p, e),
    }
}

fn is_path_safe(base_dir: &Path, requested_resource: &str) -> bool {
    let canonical_base_dir = match base_dir.canonicalize() {
        Ok(path) => path,
        Err(e) => {
            eprintln!(
                "is_path_safe: Error canonicalizing base directory '{}': {}",
                base_dir.display(),
                e
            );
            return false;
        }
    };
    let mut actual_target_path = canonical_base_dir.clone();
    for component in PathBuf::from(requested_resource.trim_start_matches('/')).components() {
        match component {
            std::path::Component::Normal(name) => {
                actual_target_path.push(name);
            }
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !actual_target_path.pop() {
                    return false;
                }
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                return false;
            }
        }
    }
    actual_target_path.starts_with(&canonical_base_dir)
}

fn handle_request(mut p: PathBuf, resource: &str, url: String) -> Cow<'static, [u8]> {
    let resource_stripped = resource.trim_start_matches("/");
    if !is_path_safe(&p, resource_stripped) {
        eprintln!("Illegal path detected: {}", resource);
        return build_error_response(Status::Forbidden);
    }
    p.push(resource_stripped);
    if p.is_dir() && resource_stripped.ends_with('/') {
        let redirect_url = format!("{}{}index.html", url, resource);
        #[cfg(debug_assertions)]
        println!("Redirecting to: {}", redirect_url);
        return build_http_response(
            Status::MovedPermamently(redirect_url),
            "text/html; charset=utf-8",
            Cow::Owned(vec![]),
        );
    }
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
            build_error_response(Status::PageNotFound)
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
    if rdr.read_line(&mut l).is_err() || l.is_empty() {
        eprintln!("Failed to read request line or empty request.");
        return;
    }
    let headers = rdr
        .lines()
        .take_while(|x| x.as_ref().map(|l| !l.is_empty()).unwrap_or(true))
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    #[cfg(debug_assertions)]
    println!("Headers: {:#?}", headers);
    let response = match headers
        .first()
        .unwrap()
        .trim()
        .split(' ')
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["GET", resource, "HTTP/1.1"] => {
            let domain_name = headers
                .iter()
                .find_map(|x| parse_host_address(x.as_str()))
                .expect("couldn't parse or find the Host header");
            let mut p = std::path::PathBuf::new();
            p.push(&resource_dir);
            if env::var("HOST_NOT_DEFINED").unwrap_or_default() != "1" {
                p.push(domain_name);
            }
            let url = format!("http://{domain_name}:{}", addr.port());
            handle_request(p, resource, url)
        }
        _ => {
            eprintln!("Unsupported or malformed request: {}", l.trim());
            build_error_response(Status::NotImplemented)
        }
    };

    if let Err(e) = stream.write_all(&response) {
        eprintln!("Failed to write response to stream: {}", e);
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
    let listener = match std::net::TcpListener::bind(saddr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Failed to bind to address {}: {}", saddr, e);
            std::process::exit(1);
        }
    };

    for stream_result in listener.incoming() {
        match stream_result {
            Ok(stream) => {
                handle_connection(&args.directory, stream, saddr);
            }
            Err(e) => {
                eprintln!("Error accepting connection: {}", e);
            }
        }
    }
}
