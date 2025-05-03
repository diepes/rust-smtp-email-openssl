use crate::state_events::Event;
use crate::stream; // Replace 'some_crate' with the actual crate or module where Stream is defined
use base64::engine::general_purpose::STANDARD as b64;
use base64::Engine; // trait
use dotenv::dotenv;
use std::env;
use std::fs;

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
    attachement_name: Option<String>,
    attachement_data: Option<Vec<u8>>,
}
impl StateMachine {
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
                    .write(
                        format!(
                            "{}\r\n",
                            b64.encode(self.smtp_connection.username.as_deref().unwrap_or(""))
                        )
                        .as_bytes(),
                    ) // send base64 encoded username
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
                    // send base64 encoded password
                    .write(
                        format!(
                            "{}\r\n",
                            b64.encode(self.smtp_connection.password.as_deref().unwrap_or(""))
                        )
                        .as_bytes(),
                    ) // base64 encoded "password"
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

    pub fn new(host: &str, port: u16) -> Self {
        StateMachine {
            state: State::Start,
            smtp_connection: stream::SmtpConnection::new(host, port, None, None),
            attachement_name: None,
            attachement_data: None,
        }
    }

    pub fn new_from_env() -> Self {
        dotenv().ok();
        let smtp_server_and_port = env::var("smtp_server").expect("smtp_server .env not set");
        let parts: Vec<&str> = smtp_server_and_port.split(':').collect();
        let (smtp_server, port) = match &parts[..] {
            [server, port] => (
                server.to_string(),
                port.parse::<u16>().expect("Invalid port number"),
            ),
            _ => panic!("Invalid format for smtp_server, expected 'server:port'"),
        };
        let smtp_username = env::var("smtp_username").expect("smtp_username .env not set");
        let smtp_password = env::var("smtp_password").expect("smtp_password .env not set");
        let from = env::var("smtp_from").expect("smtp_from .env not set");
        let to = env::var("smtp_to").expect("smtp_to .env not set");
        // debug
        let debug = env::var("smtp_debug").unwrap_or_else(|_| "false".to_string());
        let debug = match debug.as_str() {
            "true" => true,
            "True" => true,
            "TRUE" => true,
            "1" => true,
            "false" => false,
            "False" => false,
            "FALSE" => false,
            "0" => false,
            _ => panic!("Invalid value for .env smtp_debug: {}", debug),
        };
        // subject has default fallback
        let subject = env::var("smtp_subject").unwrap_or_else(|_| {
            format!(
                "Test mail Rust OpenSSL - smtp email sent with attachement at {}",
                chrono::Local::now()
            )
        });

        // Read the attachment file (e.g., a small text file or PDF)
        let smtp_attachment_path = env::var("smtp_attachment_path"); //.unwrap_or_else(|_| format!("")); // Replace with your file path
        let attachment_path: String;
        let attachment_data = if let Ok(path_provided) = smtp_attachment_path {
            attachment_path = path_provided;
            fs::read(&attachment_path)
                .expect(&format!("Failed to read attachment file {attachment_path}"))
        } else {
            attachment_path = "NO ATTACHMENT".to_string();
            vec![]
        };
        let attachment_encoded = b64.encode(&attachment_data);
        assert!(
            attachment_encoded.len() % 4 == 0,
            "Base64 output should be a multiple of 4!"
        );
        let attachment_name = &attachment_path;

        StateMachine {
            state: State::Start,
            smtp_connection: stream::SmtpConnection::new(
                &smtp_server,
                port,
                Some(&smtp_username),
                Some(&smtp_password),
            ),
            attachement_name: Some(attachment_name.to_string()),
            attachement_data: Some(attachment_data),
        }
    }
}
