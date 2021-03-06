use crate::evented::EventedDescriptor;
use ifcontrol::Iface;
use ifstructs::ifreq;
use impls::unix::*;
use libc::{c_char, c_int, dev_t, mode_t, IFF_BROADCAST, IFF_MULTICAST, S_IFCHR};
use nix::sys::socket::{socket, AddressFamily, SockFlag, SockType};
use nix::sys::stat::fstat;
use std::ffi::CString;
use std::fs::File;
use std::fs::OpenOptions;
use std::mem;
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::reactor::PollEvented2;
use TunTapError;

pub struct Native {}

impl Default for Native {
    fn default() -> Native {
        Native {}
    }
}

impl Native {
    pub fn new() -> Native {
        Native::default()
    }

    pub fn create_tun(&self) -> Result<::Virtualnterface<::Descriptor<Native>>, TunTapError> {
        let (file, name) = self.create(::VirtualInterfaceType::Tun, false)?;
        let info = Arc::new(Mutex::new(::VirtualInterfaceInfo {
            name,
            iface_type: ::VirtualInterfaceType::Tun,
        }));

        Ok(::Virtualnterface {
            queues: vec![::Descriptor::from_file(file, &info)],
            info: Arc::downgrade(&info),
        })
    }

    pub fn create_tap(&self) -> Result<::Virtualnterface<::Descriptor<Native>>, TunTapError> {
        let (file, name) = self.create(::VirtualInterfaceType::Tap, false)?;
        let info = Arc::new(Mutex::new(::VirtualInterfaceInfo {
            name,
            iface_type: ::VirtualInterfaceType::Tun,
        }));
        Ok(::Virtualnterface {
            queues: vec![::Descriptor::from_file(file, &info)],
            info: Arc::downgrade(&info),
        })
    }

    pub fn create_tun_async(&self) -> Result<::Virtualnterface<PollEvented2<EventedDescriptor<Native>>>, TunTapError> {
        let (file, name) = self.create(::VirtualInterfaceType::Tun, true)?;
        let info = Arc::new(Mutex::new(::VirtualInterfaceInfo {
            name,
            iface_type: ::VirtualInterfaceType::Tun,
        }));

        Ok(::Virtualnterface {
            queues: vec![PollEvented2::new(::Descriptor::from_file(file, &info).into())],
            info: Arc::downgrade(&info),
        })
    }

    pub fn create_tap_async(&self) -> Result<::Virtualnterface<PollEvented2<EventedDescriptor<Native>>>, TunTapError> {
        let (file, name) = self.create(::VirtualInterfaceType::Tap, true)?;
        let info = Arc::new(Mutex::new(::VirtualInterfaceInfo {
            name,
            iface_type: ::VirtualInterfaceType::Tun,
        }));
        Ok(::Virtualnterface {
            queues: vec![PollEvented2::new(::Descriptor::from_file(file, &info).into())],
            info: Arc::downgrade(&info),
        })
    }
}

// #define	TUNSIFPID	_IO('t', 95)
ioctl_none!(tun_attach_to_pid, b't', 95);
// https://github.com/nix-rust/nix/issues/934
// #define	TUNSIFHEAD	_IOW('t', 96, int)
ioctl_write_ptr!(tun_enable_header, b't', 96, c_int);
// #define	TUNSIFMODE	_IOW('t', 94, int)
ioctl_write_ptr!(tun_set_mode, b't', 94, c_int);
// #define	FIONBIO		_IOW('f', 126, int)	/* set/clear non-blocking i/o */
ioctl_write_ptr!(fd_set_non_blocking, b'f', 126, c_int);

//TODO: use ifcontrol
// #define	SIOCIFDESTROY	 _IOW('i', 121, struct ifreq)	/* destroy clone if */
ioctl_write_ptr!(ioctl_iface_destroy, b'i', 121, ifreq);

extern "C" {
    pub fn devname(dev: dev_t, mode_type: mode_t) -> *mut c_char;
}

fn get_viface_name(file: &File) -> Result<String, TunTapError> {
    let st_rdev = fstat(file.as_raw_fd()).unwrap().st_rdev;
    let device_name = unsafe { devname(st_rdev, S_IFCHR) };
    if device_name.is_null() {
        return Err(TunTapError::NotFound {
            msg: "interface not found".to_owned(),
        });
    }
    unsafe { CString::from_raw(device_name) }.into_string().map_err(|_| TunTapError::BadData {
        msg: "bad iface name returned from kernel".to_owned(),
    })
}

impl Native {
    fn create(&self, iface_type: ::VirtualInterfaceType, is_async: bool) -> Result<(File, String), TunTapError> {
        let mut clone_from_path = PathBuf::from("/dev");
        clone_from_path.push(iface_type.to_string());

        let file = OpenOptions::new().read(true).write(true).open(&clone_from_path)?;

        let name = get_viface_name(&file)?;
        let mut path = PathBuf::from("/dev");
        path.push(name.clone());

        match iface_type {
            ::VirtualInterfaceType::Tun => {
                // TUNSIFPID is supported only for tun
                unsafe { tun_attach_to_pid(file.as_raw_fd()) }?;

                let mut v = 1;
                unsafe { tun_enable_header(file.as_raw_fd(), &mut v as *mut _ as *mut _) }?;

                let mut v = IFF_BROADCAST | IFF_MULTICAST;
                unsafe { tun_set_mode(file.as_raw_fd(), &mut v as *mut _ as *mut _) }?;
            }
            ::VirtualInterfaceType::Tap => {}
        }

        if is_async {
            unsafe {
                let mut enabled = 1;
                fd_set_non_blocking(file.as_raw_fd(), &mut enabled as *mut _ as *mut _)?;
            }
        }

        Iface::find_by_name(&name)?.up()?;

        Ok((file, name))
    }
}

impl ::DescriptorCloser for Native {
    fn close_descriptor(d: &mut ::Descriptor<Native>) -> Result<(), TunTapError> {
        let name = d.info.lock().unwrap().name.clone();
        //Close underlying file at first
        mem::drop(mem::replace(&mut d.inner, File::open("/dev/null")?));

        let mut req = ifreq::from_name(&name)?;

        let ctl_fd: RawFd = socket(AddressFamily::Inet, SockType::Stream, SockFlag::empty(), None)?;

        unsafe { ioctl_iface_destroy(ctl_fd, &mut req) }?;

        Ok(())
    }
}
