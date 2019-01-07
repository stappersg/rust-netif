use std::cmp;
use std::io;
use std::ptr;

use winapi::shared::minwindef::*;
use winapi::shared::ntdef::{BOOLEAN, FALSE, HANDLE, TRUE};
use winapi::shared::winerror::*;
use winapi::um::commapi::*;
use winapi::um::fileapi::*;
use winapi::um::handleapi::*;
use winapi::um::ioapiset::*;
use winapi::um::minwinbase::*;
use winapi::um::processthreadsapi::*;
use winapi::um::winbase::COMMTIMEOUTS;
use winapi::um::winnt::*;

pub(crate) fn cvt(i: BOOL) -> io::Result<BOOL> {
    if i == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(i)
    }
}

#[derive(Debug)]
pub struct Handle(HANDLE);

unsafe impl Send for Handle {}

unsafe impl Sync for Handle {}

impl Handle {
    pub fn new(handle: HANDLE) -> Handle {
        Handle(handle)
    }

    pub fn raw(&self) -> HANDLE {
        self.0
    }

    pub fn into_raw(self) -> HANDLE {
        use std::mem;

        let ret = self.0;
        mem::forget(self);
        ret
    }

    pub fn write(&self, buf: &[u8]) -> io::Result<usize> {
        let mut bytes = 0;
        let len = cmp::min(buf.len(), <DWORD>::max_value() as usize) as DWORD;
        try!(cvt(unsafe { WriteFile(self.0, buf.as_ptr() as *const _, len, &mut bytes, 0 as *mut _) }));
        Ok(bytes as usize)
    }

    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        let mut bytes = 0;
        let len = cmp::min(buf.len(), <DWORD>::max_value() as usize) as DWORD;
        try!(cvt(unsafe { ReadFile(self.0, buf.as_mut_ptr() as *mut _, len, &mut bytes, 0 as *mut _) }));
        Ok(bytes as usize)
    }

    pub unsafe fn read_overlapped(&self, buf: &mut [u8], overlapped: *mut OVERLAPPED) -> io::Result<Option<usize>> {
        self.read_overlapped_helper(buf, overlapped, FALSE)
    }

    pub unsafe fn read_overlapped_wait(&self, buf: &mut [u8], overlapped: *mut OVERLAPPED) -> io::Result<usize> {
        match self.read_overlapped_helper(buf, overlapped, TRUE) {
            Ok(Some(bytes)) => Ok(bytes),
            Ok(None) => panic!("logic error"),
            Err(e) => Err(e),
        }
    }

    pub unsafe fn read_overlapped_helper(&self, buf: &mut [u8], overlapped: *mut OVERLAPPED, wait: BOOLEAN) -> io::Result<Option<usize>> {
        let len = cmp::min(buf.len(), <DWORD>::max_value() as usize) as DWORD;
        let res = cvt({ ReadFile(self.0, buf.as_mut_ptr() as *mut _, len, ptr::null_mut(), overlapped) });
        match res {
            Ok(_) => (),
            Err(ref e) if e.raw_os_error() == Some(ERROR_IO_PENDING as i32) => (),
            Err(e) => return Err(e),
        }

        let mut bytes = 0;
        let res = cvt({ GetOverlappedResult(self.0, overlapped, &mut bytes, wait as BOOL) });
        match res {
            Ok(_) => Ok(Some(bytes as usize)),
            Err(ref e) if e.raw_os_error() == Some(ERROR_IO_INCOMPLETE as i32) && wait == FALSE => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub unsafe fn write_overlapped(&self, buf: &[u8], overlapped: *mut OVERLAPPED) -> io::Result<Option<usize>> {
        self.write_overlapped_helper(buf, overlapped, FALSE)
    }

    pub unsafe fn write_overlapped_wait(&self, buf: &[u8], overlapped: *mut OVERLAPPED) -> io::Result<usize> {
        match self.write_overlapped_helper(buf, overlapped, TRUE) {
            Ok(Some(bytes)) => Ok(bytes),
            Ok(None) => panic!("logic error"),
            Err(e) => Err(e),
        }
    }

    unsafe fn write_overlapped_helper(&self, buf: &[u8], overlapped: *mut OVERLAPPED, wait: BOOLEAN) -> io::Result<Option<usize>> {
        let len = cmp::min(buf.len(), <DWORD>::max_value() as usize) as DWORD;
        let res = cvt({ WriteFile(self.0, buf.as_ptr() as *const _, len, ptr::null_mut(), overlapped) });
        match res {
            Ok(_) => (),
            Err(ref e) if e.raw_os_error() == Some(ERROR_IO_PENDING as i32) => (),
            Err(e) => return Err(e),
        }

        let mut bytes = 0;
        let res = cvt({ GetOverlappedResult(self.0, overlapped, &mut bytes, wait as BOOL) });
        match res {
            Ok(_) => Ok(Some(bytes as usize)),
            Err(ref e) if e.raw_os_error() == Some(ERROR_IO_INCOMPLETE as i32) && wait == FALSE => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn try_clone(&self) -> io::Result<Handle> {
        let access = 0;
        let inherit = true;
        let options = DUPLICATE_SAME_ACCESS;

        let mut ret = 0 as HANDLE;
        cvt(unsafe {
            let cur_proc = GetCurrentProcess();
            DuplicateHandle(cur_proc, self.0, cur_proc, &mut ret, access, inherit as BOOL, options)
        })?;
        Ok(Handle::new(ret))
    }

    pub fn set_no_timeouts(&self) -> io::Result<()> {
        cvt(unsafe {
            SetCommTimeouts(
                self.0,
                &mut COMMTIMEOUTS {
                    ReadIntervalTimeout: 0,
                    ReadTotalTimeoutMultiplier: 0,
                    ReadTotalTimeoutConstant: 0,
                    WriteTotalTimeoutMultiplier: 0,
                    WriteTotalTimeoutConstant: 0,
                },
            )
        })?;
        Ok(())
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}
