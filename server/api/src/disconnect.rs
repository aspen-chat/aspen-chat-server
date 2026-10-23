//! Telling a client that went away from a fault worth an operator's attention.
//!
//! Clients leave all the time: a tab closes, a phone loses its network, a browser opens a
//! speculative connection and drops it unused. Each surfaces as an error from hyper, rustls, or
//! tungstenite at whatever point the connection had reached, and logging them as errors would
//! bury the ones that matter, so callers log a disconnect at debug and anything else as before.

use std::error::Error;
use std::io;

/// Whether `e`, or any error it was caused by, says only that the peer closed, reset, or stopped
/// answering on the connection.
pub fn is_disconnect(e: &(dyn Error + 'static)) -> bool {
    let mut next = Some(e);
    while let Some(e) = next {
        if let Some(e) = e.downcast_ref::<io::Error>()
            && matches!(
                e.kind(),
                io::ErrorKind::UnexpectedEof
                    | io::ErrorKind::ConnectionReset
                    | io::ErrorKind::ConnectionAborted
                    | io::ErrorKind::BrokenPipe
                    | io::ErrorKind::NotConnected
                    | io::ErrorKind::TimedOut
            )
        {
            return true;
        }
        // A timeout here is the header read timeout or an unanswered HTTP/2 ping: a client
        // that went silent, most often one whose network is gone.
        if let Some(e) = e.downcast_ref::<hyper::Error>()
            && (e.is_incomplete_message()
                || e.is_canceled()
                || e.is_closed()
                || e.is_timeout()
                || e.is_body_write_aborted())
        {
            return true;
        }
        if let Some(e) = e.downcast_ref::<tungstenite::Error>()
            && matches!(
                e,
                tungstenite::Error::ConnectionClosed
                    | tungstenite::Error::AlreadyClosed
                    | tungstenite::Error::Protocol(
                        tungstenite::error::ProtocolError::ResetWithoutClosingHandshake
                    )
            )
        {
            return true;
        }
        next = e.source();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_resets_are_disconnects() {
        let reset = io::Error::from(io::ErrorKind::ConnectionReset);
        assert!(is_disconnect(&reset));
        let eof = tungstenite::Error::Io(io::Error::from(io::ErrorKind::UnexpectedEof));
        assert!(is_disconnect(&eof));
        let closed = axum::Error::new(tungstenite::Error::ConnectionClosed);
        assert!(is_disconnect(&closed));
        let reset = tungstenite::Error::Protocol(
            tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
        );
        assert!(is_disconnect(&reset));
    }

    #[test]
    fn other_faults_are_not() {
        assert!(!is_disconnect(&io::Error::from(
            io::ErrorKind::PermissionDenied
        )));
        let capacity =
            tungstenite::Error::Capacity(tungstenite::error::CapacityError::MessageTooLong {
                size: 2,
                max_size: 1,
            });
        assert!(!is_disconnect(&capacity));
    }
}
