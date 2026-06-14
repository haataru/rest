use std::ffi::c_void;
use std::sync::atomic::{AtomicI32, Ordering};
use std::io::Write;

#[repr(C)]
pub struct StringHeader {
    pub ref_count: AtomicI32,
    pub flags: i32, // 0 = inline memory, 1 = foreign payload
    pub len: i64,
    pub ptr: *const u8,
    pub drop_glue: Option<extern "C" fn(*mut c_void)>,
    pub foreign_box: *mut c_void,
}

#[unsafe(no_mangle)]
pub extern "C" fn rest_alloc_string(len: i64) -> *mut StringHeader {
    let size = std::mem::size_of::<StringHeader>() + len as usize;
    let ptr = unsafe { libc::malloc(size) } as *mut StringHeader;
    unsafe {
        (*ptr).ref_count.store(1, Ordering::SeqCst);
        (*ptr).flags = 0;
        (*ptr).len = len;
        (*ptr).ptr = (ptr as *const u8).add(std::mem::size_of::<StringHeader>());
        (*ptr).drop_glue = None;
        (*ptr).foreign_box = std::ptr::null_mut();
    }
    ptr
}

#[unsafe(no_mangle)]
pub extern "C" fn rest_retain_string(s: *mut StringHeader) {
    if s.is_null() { return; }
    unsafe { (*s).ref_count.fetch_add(1, Ordering::SeqCst); }
}

#[unsafe(no_mangle)]
pub extern "C" fn rest_release_string(s: *mut StringHeader) {
    if s.is_null() { return; }
    unsafe {
        if (*s).ref_count.fetch_sub(1, Ordering::SeqCst) == 1 {
            if (*s).flags == 1 {
                if let Some(glue) = (*s).drop_glue {
                    glue((*s).foreign_box);
                }
            }
            libc::free(s as *mut libc::c_void);
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rest_create_string(bytes_ptr: *const std::os::raw::c_char) -> *mut StringHeader {
    unsafe {
        let c_str = std::ffi::CStr::from_ptr(bytes_ptr);
        let bytes = c_str.to_bytes();
        let len = bytes.len() as i64;
        let ptr = rest_alloc_string(len);
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), (*ptr).ptr as *mut u8, bytes.len());
        ptr
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rest_print_string(s: *mut StringHeader) {
    if s.is_null() { return; }
    unsafe {
        let slice = std::slice::from_raw_parts((*s).ptr, (*s).len as usize);
        std::io::stdout().write_all(slice).unwrap();
        println!();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rest_strcat(a: *mut StringHeader, b: *mut StringHeader) -> *mut StringHeader {
    unsafe {
        let len_a = if a.is_null() { 0 } else { (*a).len };
        let len_b = if b.is_null() { 0 } else { (*b).len };
        let total_len = len_a + len_b;
        
        let new_s = rest_alloc_string(total_len);
        let dst_ptr = (*new_s).ptr as *mut u8;
        
        if len_a > 0 {
            std::ptr::copy_nonoverlapping((*a).ptr, dst_ptr, len_a as usize);
        }
        if len_b > 0 {
            std::ptr::copy_nonoverlapping((*b).ptr, dst_ptr.add(len_a as usize), len_b as usize);
        }
        
        new_s
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rest_streq(a: *mut StringHeader, b: *mut StringHeader) -> bool {
    if a == b { return true; }
    if a.is_null() || b.is_null() { return false; }
    unsafe {
        if (*a).len != (*b).len { return false; }
        if (*a).len == 0 { return true; }
        libc::memcmp((*a).ptr as *const libc::c_void, (*b).ptr as *const libc::c_void, (*a).len as usize) == 0
    }
}

unsafe extern "C" {
    fn __rest_alloc(size: usize) -> *mut u8;
    fn __rest_release(ptr: *mut u8);
}
