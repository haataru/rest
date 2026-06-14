use std::ffi::CStr;
use std::os::raw::c_char;
use std::sync::Mutex;

unsafe extern "C" {
    fn __rest_alloc(size: usize) -> *mut u8;
    fn __rest_release(ptr: *mut u8);
}

// Handler takes an ARC string pointer and returns an ARC string pointer
type HandlerFunc = extern "C" fn(*const c_char) -> *const c_char;

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

// Helper to convert rust string to an ARC string so we can pass it to handlers
fn create_arc_string(s: &str) -> *const c_char {
    unsafe {
        let bytes = s.as_bytes();
        let ptr = __rest_alloc(bytes.len() + 1);
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        *ptr.add(bytes.len()) = 0;
        ptr as *const c_char
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rest_start_server(port: i32) {
    let addr = format!("0.0.0.0:{}", port);
    let server = tiny_http::Server::http(&addr).unwrap();
    println!("🚀 Rest Web-Magic Server running on http://{}", addr);
    
    for mut request in server.incoming_requests() {
        let method = request.method().as_str().to_string();
        let path = request.url().to_string();
        
        let mut matched_handler = None;
        {
            let routes = ROUTES.lock().unwrap();
            for (r_method, r_path, r_handler) in routes.iter() {
                if r_method.eq_ignore_ascii_case(&method) && r_path == &path {
                    matched_handler = Some(*r_handler);
                    break;
                }
            }
        }
        
        if let Some(handler) = matched_handler {
            let mut content = String::new();
            let _ = request.as_reader().read_to_string(&mut content);
            
            unsafe {
                // Pass body as an ARC string
                let arc_body = create_arc_string(&content);
                
                // Call the LLVM-generated handler
                let res_ptr = handler(arc_body);
                
                // The result is an ARC string
                let res_str = if !res_ptr.is_null() {
                    CStr::from_ptr(res_ptr).to_string_lossy().into_owned()
                } else {
                    String::new()
                };
                
                // In our ARC model, handler returned an owned reference, so we release it here.
                if !res_ptr.is_null() {
                    __rest_release(res_ptr as *mut u8);
                }
                
                // We don't need to release arc_body because the LLVM handler is compiled
                // with ARC rules: arguments passed by value are released by the callee
                // at the end of its scope! (Actually, string is passed by ref. Wait!)
                // In Rest ARC, a variable owns its reference. An argument variable
                // also owns its reference. When we call `handler(arc_body)`, it takes ownership
                // of the retain count we implicitly gave it. The `__rest_alloc` sets retain count to 1.
                // When `handler` returns, its parameters are popped and `__rest_release` is called on them!
                // So `arc_body` WILL BE FREED by the handler. Perfect!
                
                let response = tiny_http::Response::from_string(res_str)
                    .with_header(tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap());
                let _ = request.respond(response);
            }
        } else {
            let response = tiny_http::Response::from_string("Not Found").with_status_code(404);
            let _ = request.respond(response);
        }
    }
}
