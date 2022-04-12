use super::buffer::Buffer;
use super::event_loop::EventLoop;
use super::event_loop::*;
use log::{debug, warn};
use nix::fcntl::{fcntl, open, FcntlArg, OFlag};
use nix::sys::epoll::EpollFlags;
use nix::sys::sendfile::sendfile;
use nix::sys::socket::shutdown;
use nix::sys::socket::{accept4, connect, getpeername, getsockname, setsockopt, socket, sockopt};
use nix::sys::socket::{AddressFamily, InetAddr, Shutdown};
use nix::sys::socket::{SockAddr, SockFlag, SockProtocol, SockType};
use nix::sys::stat::{fstat, Mode};
use nix::unistd::write;
use std::net::{SocketAddr, TcpListener};
use std::os::unix::prelude::AsRawFd;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

pub type ConnRef = Arc<Mutex<Connection>>;
#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub enum State {
    Reading,
    Ready,
    Writing,
    Finished,
    Closed,
}

const READABLE: u8 = 0b0001;
const WRITABLE: u8 = 0b0010;

pub trait EventSet {
    fn is_readable(&self) -> bool;
    fn is_writeable(&self) -> bool;
    fn is_close(&self) -> bool;
    fn is_error(&self) -> bool;
    fn is_hup(&self) -> bool;
}
impl EventSet for EpollFlags {
    fn is_readable(&self) -> bool {
        (*self & (EpollFlags::EPOLLIN | EpollFlags::EPOLLPRI)).bits() > 0
    }
    fn is_writeable(&self) -> bool {
        (*self & EpollFlags::EPOLLOUT).bits() > 0
    }
    fn is_close(&self) -> bool {
        (*self & EpollFlags::EPOLLHUP).bits() > 0 && !((*self & EpollFlags::EPOLLIN).bits() > 0)
    }
    fn is_error(&self) -> bool {
        (*self & EpollFlags::EPOLLERR).bits() > 0
    }
    fn is_hup(&self) -> bool {
        (*self & EpollFlags::EPOLLHUP).bits() > 0
    }
}

#[derive(Debug, Clone)]
pub struct Connection {
    fd: i32,
    state: State,
    input_buf: Buffer,
    output_buf: Buffer,
    local_addr: String,
    peer_addr: String,
    revents: EpollFlags,
}

impl Connection {
    pub fn new(fd: i32) -> Self {
        assert!(fd > 0);
        let local_addr = format!("{}", getsockname(fd).unwrap());
        let peer_addr = format!("{}", getpeername(fd).unwrap());
        Connection {
            fd,
            state: State::Ready,
            input_buf: Buffer::new(),
            output_buf: Buffer::new(),
            local_addr,
            peer_addr,
            revents: EpollFlags::empty(),
        }
    }
    pub fn bind(addr: &str) -> (i32, TcpListener) {
        let listener = TcpListener::bind(addr).unwrap();
        (listener.as_raw_fd(), listener)
    }
    pub fn connect(addr: &str) -> Connection {
        let sockfd = socket(
            AddressFamily::Inet,
            SockType::Stream,
            SockFlag::SOCK_CLOEXEC,
            SockProtocol::Tcp,
        )
        .unwrap();

        let addr = SocketAddr::from_str(addr).unwrap();
        let inet_addr = InetAddr::from_std(&addr);
        let sock_addr = SockAddr::new_inet(inet_addr);
        // TODO: add a exception handle
        match connect(sockfd, &sock_addr) {
            Ok(()) => debug!("a new connection: {}", sockfd),
            Err(e) => warn!("connect failed: {}", e),
        }
        return Connection::new(sockfd);
    }
    pub fn accept(listen_fd: i32) -> Self {
        let fd = accept4(listen_fd, SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK).unwrap();
        setsockopt(fd, sockopt::TcpNoDelay, &true).unwrap();
        setsockopt(fd, sockopt::KeepAlive, &true).unwrap();
        Connection::new(fd)
    }
    pub fn set_no_delay(&mut self, on: bool) {
        setsockopt(self.fd, sockopt::KeepAlive, &on).unwrap();
    }
    pub fn set_revents(&mut self, revents: &EpollFlags) {
        self.revents = revents.clone();
    }
    pub fn get_revents(&self) -> EpollFlags {
        self.revents
    }
    pub fn connected(&self) -> bool {
        self.state != State::Closed
    }
    pub fn get_peer_addr(&self) -> String {
        self.peer_addr.clone()
    }
    pub fn get_local_addr(&self) -> String {
        self.local_addr.clone()
    }
    pub fn dispatch(&mut self, revents: EpollFlags) -> State {
        self.state = State::Ready;
        if revents.is_readable() {
            self.input_buf.read(self.fd);
        }
        if revents.is_writeable() {
            // self.write();
        }
        if revents.is_error() {
            self.state = State::Closed;
        }
        if revents.is_close() {
            self.state = State::Closed;
        }
        return self.state;
    }
    pub fn get_fd(&self) -> i32 {
        self.fd
    }
    pub fn get_state(&self) -> State {
        self.state
    }
    pub fn register_read(&mut self, event_loop: &mut EventLoop) {
        // self.read_buf.clear();
        event_loop.reregister(
            self.fd,
            EVENT_HUP | EVENT_ERR | EVENT_WRIT | EVENT_READ | EVENT_LEVEL,
        );
    }
    pub fn deregister(&mut self, event_loop: &mut EventLoop) {
        event_loop.deregister(self.fd);
        self.shutdown();
    }
    pub fn shutdown(&mut self) {
        self.state = State::Closed;
        match shutdown(self.fd, Shutdown::Both) {
            Ok(()) => (),
            Err(e) => warn!("Shutdown {} occur {} error", self.fd, e),
        }
    }
    // TODO: 限速发送，定时发送一部分
    pub fn send_file(&mut self, file: &str) -> Option<usize> {
        let fd = open(file, OFlag::O_RDWR, Mode::S_IRUSR).unwrap();
        let stat = fstat(fd).unwrap();
        let size = sendfile(self.fd, fd, None, stat.st_size as usize).unwrap();
        Some(size)
    }
    pub fn send(&mut self, buf: &[u8]) {
        match write(self.fd, buf) {
            Ok(n) => debug!("Send data len: {}", n),
            Err(e) => warn!("Send data error: {}", e),
        };
    }
    pub fn read_buf(&mut self) -> Vec<u8> {
        self.input_buf.read(self.fd);
        self.input_buf.read_buf()
    }
    pub fn read_msg(&mut self) -> Option<Vec<u8>> {
        match self.input_buf.read(self.fd) {
            Some(0) | None => None,
            Some(_) => self.input_buf.get_crlf_line(),
        }
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        if 0 > fcntl(self.fd, FcntlArg::F_GETFL).unwrap() {
            self.shutdown();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix::sys::socket::socketpair;
    use std::cell::RefCell;
    use std::rc::Rc;
    #[test]
    fn test_send_rev_msg() {
        let (rev, send) = socketpair(
            AddressFamily::Inet,
            SockType::Stream,
            SockProtocol::Tcp,
            SockFlag::SOCK_CLOEXEC,
        )
        .unwrap();
        let rev = Rc::new(RefCell::new(Connection::new(rev)));
        let send = Rc::new(RefCell::new(Connection::new(send)));
        assert_eq!((*rev.borrow_mut()).connected(), true);
        assert_eq!((*send.borrow_mut()).connected(), true);

        // *send.borrow_mut().send("");
    }
    #[test]
    fn test_send_rev_file() {}
}
