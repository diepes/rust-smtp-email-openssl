use crate::state_events::Event;
mod send_body;
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
    ConnectedTls,
    SendingMailHeaders,
    SendingMailData,
    MailSent,
    Finished,
    Failed,
}
pub struct StateMachine {
    pub state: State,
    pub smtp_connection: stream::SmtpConnection,
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
                log::info!("Transitioning from ConnectingTcp to ConnectedTcpHelloSent, send EHLO");
                self.write_and_get_next_state(
                    "EHLO rustclient",
                    State::ConnectedTcpHelloSent,
                    "EHLO sent successfully",
                    State::Failed,
                )
                .await
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
                self.write_and_get_next_state(
                    "STARTTLS",
                    State::ConnectedTcpStartTls,
                    "STARTTLS sent successfully",
                    State::Failed,
                )
                .await
            }
            (State::ConnectedTcpStartTls, Event::Received220(_msg)) => {
                log::info!("STARTTLS accepted server ready to transition to Tls");
                match self.smtp_connection.switch_to_tls().await {
                    Ok(_) => {
                        self.write_and_get_next_state(
                            "EHLO rustclient",
                            State::ConnectedTls,
                            "EHLO over TLS sent successfully",
                            State::Failed,
                        )
                        .await
                    }
                    Err(e) => {
                        log::error!("Failed to switch to TLS: {:?}", e);
                        State::Failed
                    }
                }
            }
            (State::ConnectedTls, Event::Received250(_msg)) => {
                log::info!("TLS 2nd EHLO accepted, starting AUTH");
                self.write_and_get_next_state(
                    "AUTH LOGIN",
                    State::ConnectedTls,
                    "AUTH LOGIN sent successfully",
                    State::Failed,
                )
                .await
            }
            (State::ConnectedTls, Event::Received250StartTlsAuth(_msg)) => {
                log::info!("Received request to proceed with AUTH");
                // send "AUTH LOGIN"
                self.write_and_get_next_state(
                    "AUTH LOGIN",
                    State::ConnectedTls,
                    "AUTH LOGIN sent successfully",
                    State::Failed,
                )
                .await
            }
            (State::ConnectedTls, Event::Received334Username) => {
                log::info!("Received request for username");
                // send username
                if let Some(username) = self.smtp_connection.username.clone() {
                    self.write_and_get_next_state(
                        &b64.encode(&username),
                        State::ConnectedTls,
                        "Username sent successfully",
                        State::Failed,
                    )
                    .await
                } else {
                    log::error!("Username not provided ?");
                    State::Failed
                }
            }
            (State::ConnectedTls, Event::Received334Password) => {
                log::info!("Received request for password");
                // send password
                if let Some(password) = self.smtp_connection.password.clone() {
                    self.write_and_get_next_state(
                        &b64.encode(password),
                        State::ConnectedTls,
                        "Password sent successfully",
                        State::Failed,
                    )
                    .await
                } else {
                    log::error!("Password not provided ?");
                    State::Failed
                }
            }
            (State::ConnectedTls, Event::AuthSuccess(_)) => {
                log::info!("AUTH successfull, ready to start sending MAIL FROM");
                self.write_and_get_next_state(
                    &format!("MAIL FROM:<{}>", self.smtp_connection.from),
                    State::SendingMailHeaders,
                    "MAIL FROM sent successfully",
                    State::Failed,
                )
                .await
            }

            (State::SendingMailHeaders, Event::Received250SenderOk(_msg)) => {
                log::info!("MAIL FROM accepted, ready to send RCPT TO");
                self.write_and_get_next_state(
                    &format!("RCPT TO:<{}>", self.smtp_connection.to),
                    State::SendingMailHeaders,
                    "RCPT TO sent successfully",
                    State::Failed,
                )
                .await
            }
            (State::SendingMailHeaders, Event::Received250RecipientOk(_msg)) => {
                log::info!("RCPT TO accepted, request we start sending DATA");
                self.write_and_get_next_state(
                    "DATA",
                    State::SendingMailData,
                    "DATA sent successfully",
                    State::Failed,
                )
                .await
            }

            (State::SendingMailData, Event::Received354MailInput(_msg)) => {
                log::info!("DATA accepted, ready to send email body");
                // Send the email body
                send_body::send_body(&mut self.smtp_connection).await
            }
            (State::MailSent, Event::Received250Queued(_msg)) => {
                log::info!("Email sent successfully, transitioning to Finished");
                self.write_and_get_next_state("QUIT", State::Finished, "QUIT", State::Failed)
                    .await
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

    // Helper funtion to write to stream and return next state ok or error
    async fn write_and_get_next_state(
        &mut self,
        data: &str,
        state_ok: State,
        msg_ok: &str,
        state_error: State,
    ) -> State {
        log::info!("Sending ... {}\\r\\n", data);
        match self
            .smtp_connection
            // send base64 encoded password
            .write(format!("{}\r\n", data).as_bytes()) // base64 encoded "password"
            .await
        {
            Ok(_) => {
                log::info!("{}", msg_ok);
                state_ok
            }
            Err(e) => {
                log::error!("Failed to write to stream: {:?}", e);
                state_error
            }
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
            smtp_connection: stream::SmtpConnection {
                smtp_stream: stream::Stream::None,
                host: smtp_server,
                port,
                username: Some(smtp_username),
                password: Some(smtp_password),
                attachement_name: Some(attachment_name.to_string()),
                attachement_data: Some(attachment_data),
                from: from,
                to: to,
                subject: subject,
            },
        }
    }
}
