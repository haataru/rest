use std::ffi::CStr;
use std::os::raw::c_char;
use std::sync::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::ffi::c_void;
use std::net::{TcpListener, TcpStream};
use std::io::{Read, Write};
use std::thread;

use crate::core::*;

// Handler takes an ARC StringHeader pointer and returns an ARC StringHeader pointer
type HandlerFunc = extern "C" fn(*mut StringHeader) -> *mut StringHeader;

lazy_static::lazy_static! {
    static ref ROUTES: Mutex<Vec<(String, String, HandlerFunc)>> = Mutex::new(Vec::new());
}

#[unsafe(no_mangle)]
pub extern "C" fn rest_register_route(method: *const c_char, path: *const c_char, handler: HandlerFunc) {
    unsafe {
        let method_str = CStr::from_ptr(method).to_string_lossy().into_owned();
        let path_str = CStr::from_ptr(path).to_string_lossy().into_owned();
        ROUTES.lock().unwrap().push((method_str, path_str, handler));
    }
}

fn handle_client(mut stream: TcpStream, routes: Arc<HashMap<(String, String), HandlerFunc>>) {
    let mut buffer = [0; 4096];
    if let Ok(bytes_read) = stream.read(&mut buffer) {
        if bytes_read == 0 { return; }
        let request = String::from_utf8_lossy(&buffer[..bytes_read]);
        let mut lines = request.lines();
        if let Some(first_line) = lines.next() {
            let parts: Vec<&str> = first_line.split_whitespace().collect();
            if parts.len() >= 2 {
                let method = parts[0].to_uppercase();
                let path = parts[1].to_string();

                if let Some(&handler) = routes.get(&(method, path)) {
                    // Extract body if any
                    let mut body_str = String::new();
                    if let Some(idx) = request.find("\r\n\r\n") {
                        body_str = request[idx + 4..].to_string();
                    }

                    unsafe {
                        let c_body = std::ffi::CString::new(body_str).unwrap();
                        let body_bytes = c_body.as_bytes_with_nul();
                        let arc_body = rest_create_string(body_bytes.as_ptr() as *const c_char);
                        
                        let res_ptr = handler(arc_body);
                        
                        let res_str = if !res_ptr.is_null() {
                            let slice = std::slice::from_raw_parts((*res_ptr).ptr, (*res_ptr).len as usize);
                            String::from_utf8_lossy(slice).into_owned()
                        } else {
                            String::new()
                        };
                        
                        if !res_ptr.is_null() {
                            rest_release_string(res_ptr);
                        }
                        
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                            res_str.len(), res_str
                        );
                        let _ = stream.write_all(response.as_bytes());
                    }
                } else {
                    let response = "HTTP/1.1 404 NOT FOUND\r\nContent-Length: 9\r\n\r\nNot Found";
                    let _ = stream.write_all(response.as_bytes());
                }
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rest_start_server(port: i32) {
    let routes_map = {
        let mut routes = ROUTES.lock().unwrap();
        let mut map = HashMap::new();
        for (method, path, handler) in routes.drain(..) {
            map.insert((method.to_uppercase(), path), handler);
        }
        map
    };
    let routes_arc = Arc::new(routes_map);

    let addr = format!("0.0.0.0:{}", port);
    let listener = TcpListener::bind(&addr).unwrap();
    println!("🚀 Rest Web-Magic Server running on http://{} (Raw Sockets)", addr);
    
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let routes = Arc::clone(&routes_arc);
                thread::spawn(move || {
                    handle_client(stream, routes);
                });
            }
            Err(e) => {
                eprintln!("Accept error: {:?}", e);
            }
        }
    }
}

