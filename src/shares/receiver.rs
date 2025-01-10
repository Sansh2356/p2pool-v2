use std::error::Error;
use zmq;
use serde_json::Value;
use tracing::{error, info};

const ENDPOINT: &str = "tcp://localhost:8881";

// Define a trait for the socket operations we need
// Use a trait to enable testing with a mock socket
trait ShareSocket {
    fn recv_string(&self) -> Result<Result<String, Vec<u8>>, zmq::Error>;
}

// Implement the trait for the real ZMQ socket
impl ShareSocket for zmq::Socket {
    fn recv_string(&self) -> Result<Result<String, Vec<u8>>, zmq::Error> {
        self.recv_string(0)
    }
}

// Function to create the real ZMQ socket
fn create_zmq_socket() -> Result<zmq::Socket, Box<dyn Error>> {
    let ctx = zmq::Context::new();
    let socket = ctx.socket(zmq::SUB)?;
    socket.connect(ENDPOINT)?;
    socket.set_subscribe(b"")?;
    Ok(socket)
}

// Generic function to receive shares from any ShareSocket
// This is generic to enable testing with a mock socket
fn receive_shares<S: ShareSocket>(
    socket: &S,
    tx: tokio::sync::mpsc::Sender<Value>
) -> Result<(), Box<dyn Error>> {
    loop {
        match socket.recv_string() {
            Ok(Ok(json_str)) => {
                match serde_json::from_str(&json_str) {
                    Ok(json_value) => {
                        info!("Received share: {}", json_str);
                        if let Err(e) = tx.blocking_send(json_value) {
                            error!("Failed to send share to channel: {}", e);
                        }
                    },
                    Err(e) => {
                        error!("Failed to parse JSON: {}", e);
                        return Err(Box::new(e));
                    }
                }
            },
            Ok(Err(e)) => {
                error!("Failed to decode message: {:?}", e);
                return Err(Box::new(zmq::Error::EINVAL));
            },
            Err(e) => {
                error!("Failed to receive message: {:?}", e);
                return Err(Box::new(e));
            }
        }
    }
}

// A receive function that clients use to receive shares
// This function creates the ZMQ socket and passes it to the receive_shares function
pub fn receive(tx: tokio::sync::mpsc::Sender<Value>) -> Result<(), Box<dyn Error>> {
    let socket = create_zmq_socket()?;
    receive_shares(&socket, tx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    // Mock socket for testing
    struct MockSocket {
        messages: Vec<Result<Result<String, Vec<u8>>, zmq::Error>>,
        current: usize,
    }

    impl MockSocket {
        fn new(messages: Vec<Result<Result<String, Vec<u8>>, zmq::Error>>) -> Self {
            Self {
                messages,
                current: 0,
            }
        }
    }

    impl ShareSocket for MockSocket {
        fn recv_string(&self) -> Result<Result<String, Vec<u8>>, zmq::Error> {
            if self.current >= self.messages.len() {
                panic!("No more mock messages");
            }
            self.messages[self.current].clone()
        }
    }

    #[tokio::test]
    async fn test_receive_valid_json() {
        let (tx, mut rx) = mpsc::channel(100);
        
        let mock_messages = vec![
            Ok(Ok(r#"{"share": "test", "value": 123}"#.to_string())),
        ];
        let mock_socket = MockSocket::new(mock_messages);

        // Spawn the receive_shares function in a separate task
        tokio::spawn(async move {
            receive_shares(&mock_socket, tx).unwrap();
        });

        // Receive the message from the channel
        if let Some(value) = rx.recv().await {
            assert_eq!(value["share"], "test");
            assert_eq!(value["value"], 123);
        }
    }

    #[tokio::test]
    async fn test_receive_invalid_json() {
        let (tx, _rx) = mpsc::channel(100);
        
        let mock_messages = vec![
            Ok(Ok("invalid json".to_string())),
        ];
        let mock_socket = MockSocket::new(mock_messages);

        let result = receive_shares(&mock_socket, tx);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_receive_decode_error() {
        let (tx, _rx) = mpsc::channel(100);
        
        let mock_messages = vec![
            Ok(Err(vec![1, 2, 3])), // Simulating decode error
        ];
        let mock_socket = MockSocket::new(mock_messages);

        let result = receive_shares(&mock_socket, tx);
        assert!(result.is_err());
    }
}