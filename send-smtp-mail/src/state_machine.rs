use crate::state_events::Event;
use crate::stream; // Replace 'some_crate' with the actual crate or module where Stream is defined

#[derive(Debug, PartialEq, Clone)]
pub enum State {
    Start,
    ConnectingTcp,
    ConnectedTcpHelloSent,
    ConnectedTcp,
    ConnectedTcpStartTls,
    HelloAccepted,
    ConnectedTls,
    Finished,
    Failed,
}
pub struct StateMachine {
    pub state: State,
    pub smtp_connection: stream::SmtpConnection,
}
impl StateMachine {
    pub fn new(host: &str, port: u16) -> Self {
        StateMachine {
            state: State::Start,
            smtp_connection: stream::SmtpConnection::new(host, port),
        }
    }

    pub async fn handle_event(&mut self, event: Event) {
        log::warn!(
            "State Machine State: {:?} got event: {:?}",
            self.state,
            event
        );
        self.state = match (&self.state, event) {
            (State::Start, Event::Connect) => {
                match self
                    .smtp_connection
                    .connect_to_server() // Call the function to connect to the server
                    .await
                {
                    Ok(_) => {
                        log::info!("Transitioning from Connect to ConnectedReady");
                        State::ConnectingTcp
                    }
                    Err(e) => {
                        log::error!("Failed to connect: {:?}", e);
                        State::Failed
                    }
                }
            }
            (State::ConnectingTcp, Event::Received220(_msg)) => {
                log::info!("Transitioning from ConnectingTcp to ConnectedTcp, send EHLO");
                match self
                    .smtp_connection
                    .write("EHLO rustclien\r\n".as_bytes())
                    .await
                {
                    Ok(_) => {
                        log::info!("EHLO sent successfully");
                        State::ConnectedTcpHelloSent
                    }
                    Err(e) => {
                        log::error!("Failed to send EHLO: {:?}", e);
                        State::Failed
                    }
                }
            }
            (State::ConnectedTcpHelloSent, Event::Received250(_msg)) => {
                log::info!("EHLO accepted, transitioning to ConnectedTcp");
                State::ConnectedTcp
            }
            (
                State::ConnectedTcp | State::ConnectedTcpHelloSent,
                Event::Received250StartTls(_msg),
            ) => {
                log::info!("Sending STARTTLS command");
                match self.smtp_connection.write("STARTTLS\r\n".as_bytes()).await {
                    Ok(_) => {
                        log::info!("STARTTLS sent successfully");
                        State::ConnectedTcpStartTls
                    }
                    Err(e) => {
                        log::error!("Failed to send STARTTLS: {:?}", e);
                        State::Failed
                    }
                }
            }
            (State::ConnectedTcpStartTls, Event::Received220(_msg)) => {
                log::info!("STARTTLS accepted, transitioning to ConnectedTls");
                match self.smtp_connection.switch_to_tls().await {
                    Ok(_) => {
                        log::info!("TLS handshake completed successfully send 2nd EHLO");
                        match self
                            .smtp_connection
                            .write("EHLO rustclien\r\n".as_bytes())
                            .await
                        {
                            Ok(_) => {
                                log::info!("EHLO sent successfully");
                                State::ConnectedTls
                            }
                            Err(e) => {
                                log::error!("Failed to send EHLO: {:?}", e);
                                State::Failed
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to switch to TLS: {:?}", e);
                        State::Failed
                    }
                }
            }
            (State::ConnectedTls, Event::Received250StartTlsAuth(_msg)) => {
                log::info!("Received request to proceed with AUTH");
                // send "AUTH LOGIN"
                match self
                    .smtp_connection
                    .write("AUTH LOGIN\r\n".as_bytes())
                    .await
                {
                    Ok(_) => {
                        log::info!("AUTH LOGIN sent successfully");
                        State::ConnectedTls
                    }
                    Err(e) => {
                        log::error!("Failed to send AUTH LOGIN: {:?}", e);
                        State::Failed
                    }
                }
            }
            (State::ConnectedTls, Event::Received334Username) => {
                log::info!("Received request for username");
                // send username
                match self
                    .smtp_connection
                    .write("dXNlcm5hbWU=\r\n".as_bytes()) // base64 encoded "username"
                    .await
                {
                    Ok(_) => {
                        log::info!("Username sent successfully");
                        State::ConnectedTls
                    }
                    Err(e) => {
                        log::error!("Failed to send username: {:?}", e);
                        State::Failed
                    }
                }
            }
            (State::ConnectedTls, Event::Received334Password) => {
                log::info!("Received request for password");
                // send password
                match self
                    .smtp_connection
                    .write("cGFzc3dvcmQ=\r\n".as_bytes()) // base64 encoded "password"
                    .await
                {
                    Ok(_) => {
                        log::info!("Password sent successfully");
                        State::ConnectedTls
                    }
                    Err(e) => {
                        log::error!("Failed to send password: {:?}", e);
                        State::Failed
                    }
                }
            }
            (_, Event::Received5xx(_msg)) => {
                log::error!("Received 5xx error, transitioning to Failed");
                State::Failed
            }
            (_, Event::Complete) => {
                log::info!("Transitioning from ?? to Finished");
                State::Finished
            }
            _ => {
                log::error!("No valid transition for this event.");
                State::Failed
            }
        }
    }
}
