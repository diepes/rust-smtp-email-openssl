use base64::engine::general_purpose::STANDARD as b64;
use base64::Engine; // trait

use crate::stream;

#[derive(Debug, PartialEq)]
pub enum Event {
    NoEvent,
    Connect,
    Received220(String),
    Received250(String),
    Received250StartTls(String),
    Received250StartTlsAuth(String),
    Received334Username,
    Received334Password,
    Received4xx(String),
    Received5xx(String),
    Stop,
    Timeout,
    Complete,
}
pub async fn get_event(smtp_connection: &mut stream::SmtpConnection) -> Event {
    // Placeholder for actual event logic
    // This is where you would implement the logic to determine the event based on the state of the connection
    let input = smtp_connection.read().await;
    log::info!("Input read debug: {:?}", input);
    match input {
        Ok(input) => {
            log::debug!("Received data from SMTP server: {}", input);
            for line in input.lines() {
                if line.starts_with("220") {
                    log::info!("starts_with 220: {}", line);
                    return Event::Received220(line.to_string());
                };
                if line.starts_with("250") {
                    // Check if the line contains "STARTTLS"
                    if input.contains("STARTTLS") {
                        log::info!("STARTTLS supported: {}", line);
                        return Event::Received250StartTls(line.to_string());
                    }
                    if input.contains("AUTH") {
                        log::info!("AUTH supported: {}", line);
                        return Event::Received250StartTlsAuth(line.to_string());
                    }
                    log::info!("starts_with 250: {}", line);
                    return Event::Received250(line.to_string());
                };
                if line.starts_with(&format!("334 {}", b64.encode("Username:"))) {
                    log::info!("starts_with 334: {}", line);
                    return Event::Received334Username;
                };
                if line.starts_with(&format!("334 {}", b64.encode("Password:"))) {
                    log::info!("starts_with 334: {}", line);
                    return Event::Received334Password;
                };
                if line.starts_with("4") {
                    log::info!("starts_with 4xx: {}", line);
                    return Event::Received4xx(line.to_string());
                };
                if line.starts_with("5") {
                    return Event::Received5xx(line.to_string());
                }
            }
            return Event::NoEvent;
        }
        Err(e) => {
            log::error!("Error reading from SMTP server: {:?}", e);
            return Event::Stop;
        }
    }
}
