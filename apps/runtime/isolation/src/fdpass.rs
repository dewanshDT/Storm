//! One line each way, with at most one descriptor beside it (`SCM_RIGHTS`):
//! the host hands over a session's terminal with `start`, and the helper
//! hands back a pidfd for the session's leader (136d, 136i).

use std::io::{self, IoSlice, IoSliceMut};
use std::mem::MaybeUninit;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};

use rustix::net::{
    RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags, SendAncillaryBuffer,
    SendAncillaryMessage, SendFlags,
};

/// Sends `bytes` whole, with `fd` attached to the first part.
pub fn send_line(socket: impl AsFd, bytes: &[u8], fd: Option<BorrowedFd<'_>>) -> io::Result<()> {
    let socket = socket.as_fd();
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut control = SendAncillaryBuffer::new(&mut space);
    let fds;
    if let Some(fd) = fd {
        fds = [fd];
        control.push(SendAncillaryMessage::ScmRights(&fds));
    }
    let mut sent = rustix::net::sendmsg(
        socket,
        &[IoSlice::new(bytes)],
        &mut control,
        SendFlags::NOSIGNAL,
    )?;
    while sent < bytes.len() {
        sent += rustix::net::send(socket, &bytes[sent..], SendFlags::NOSIGNAL)?;
    }
    Ok(())
}

/// What arrived: the line without its newline, and every descriptor that
/// came with it (the caller refuses more than it expects).
pub struct Received {
    pub line: Vec<u8>,
    pub fds: Vec<OwnedFd>,
    /// The line went past `max` bytes before its newline.
    pub too_large: bool,
}

/// Reads up to a newline, end of stream or `max` bytes, keeping every
/// descriptor that arrives on the way. Descriptors are close-on-exec as
/// they land, and a control message cut short refuses the whole read.
pub fn recv_line(socket: impl AsFd, max: usize) -> io::Result<Received> {
    let socket = socket.as_fd();
    let mut line = Vec::new();
    let mut fds = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(2))];
        let mut control = RecvAncillaryBuffer::new(&mut space);
        let got = rustix::net::recvmsg(
            socket,
            &mut [IoSliceMut::new(&mut buf)],
            &mut control,
            RecvFlags::CMSG_CLOEXEC,
        )?;
        for msg in control.drain() {
            if let RecvAncillaryMessage::ScmRights(received) = msg {
                fds.extend(received);
            }
        }
        if got.flags.contains(ReturnFlags::CTRUNC) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "too many descriptors",
            ));
        }
        let chunk = &buf[..got.bytes];
        if let Some(end) = chunk.iter().position(|&b| b == b'\n') {
            line.extend_from_slice(&chunk[..end]);
            let too_large = line.len() > max;
            return Ok(Received {
                line,
                fds,
                too_large,
            });
        }
        line.extend_from_slice(chunk);
        if got.bytes == 0 || line.len() > max {
            let too_large = line.len() > max;
            return Ok(Received {
                line,
                fds,
                too_large,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    #[test]
    fn a_line_and_its_descriptor_arrive_together() {
        let (a, b) = UnixStream::pair().unwrap();
        let (mut r, w) = std::io::pipe().unwrap();
        send_line(&a, b"{\"op\":\"start\"}\n", Some(w.as_fd())).unwrap();
        let got = recv_line(&b, 1024).unwrap();
        assert_eq!(got.line, b"{\"op\":\"start\"}");
        assert_eq!(got.fds.len(), 1);
        let flags = rustix::io::fcntl_getfd(&got.fds[0]).unwrap();
        assert!(flags.contains(rustix::io::FdFlags::CLOEXEC));
        // The same pipe: what goes into the received end comes out of ours.
        drop(w);
        let mut sent = std::fs::File::from(got.fds.into_iter().next().unwrap());
        sent.write_all(b"through").unwrap();
        drop(sent);
        let mut out = String::new();
        r.read_to_string(&mut out).unwrap();
        assert_eq!(out, "through");
    }

    #[test]
    fn a_line_without_a_descriptor_has_none() {
        let (a, b) = UnixStream::pair().unwrap();
        send_line(&a, b"{\"op\":\"lease\"}\n", None).unwrap();
        let got = recv_line(&b, 1024).unwrap();
        assert!(got.fds.is_empty() && !got.too_large);
        assert_eq!(got.line, b"{\"op\":\"lease\"}");
    }

    #[test]
    fn more_descriptors_than_room_refuse_the_read() {
        let (a, b) = UnixStream::pair().unwrap();
        let (_r, w) = std::io::pipe().unwrap();
        let fds = [w.as_fd(); 16];
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(16))];
        let mut control = SendAncillaryBuffer::new(&mut space);
        assert!(control.push(SendAncillaryMessage::ScmRights(&fds)));
        rustix::net::sendmsg(
            &a,
            &[IoSlice::new(b"x\n")],
            &mut control,
            SendFlags::empty(),
        )
        .unwrap();
        assert!(recv_line(&b, 1024).is_err());
    }

    #[test]
    fn an_endless_line_stops_at_the_limit() {
        let (mut a, b) = UnixStream::pair().unwrap();
        let writer = std::thread::spawn(move || {
            let _ = a.write_all(&vec![b'x'; 64 * 1024]);
        });
        let got = recv_line(&b, 1000).unwrap();
        assert!(got.too_large);
        assert!(got.line.len() <= 1000 + 4096);
        drop(b);
        writer.join().unwrap();
    }
}
