use crate::state_machine::State;
use crate::stream::SmtpConnection; // Import State from the appropriate module

use base64::engine::general_purpose::STANDARD as b64;
use base64::Engine; // trait

pub async fn send_body(smtp: &mut SmtpConnection) -> State {
    // log4::init_log();
    // Send the email body
    log::info!("Sending email body...");
    let boundary = "boundary";
    let attachment_data = smtp.attachement_data.clone();
    let attachment_encoded = b64.encode(&attachment_data.unwrap_or_default());
    assert!(
        attachment_encoded.len() % 4 == 0,
        "Base64 output should be a multiple of 4!"
    );
    // Send the email headers and body
    let data_header = format!(
        "From: {from}\r\n\
            To: {to}\r\n\
            Subject: {subject}\r\n\
            MIME-Version: 1.0\r\n\
            Content-Type: multipart/mixed; boundary=\"{boundary}\"\r\n\
            \r\n\
            --{boundary}\r\n\
            Content-Type: text/plain; charset=utf-8\r\n",
        from = smtp.from.clone(),
        to = smtp.to.clone(),
        subject = smtp.subject.clone(),
        boundary = boundary,
    );
    let data_msg = format!(
        "This is the email body.\r\n\
            \r\n\
            Was sent from {from} to {to}.\r\n\
            \r\n\
            Subject: \"{subject}\"\r\n\
            \r\n\
            See the attached file!\r\n\
            \r\n\
            --{boundary}\r\n",
        from = smtp.from.clone(),
        to = smtp.to.clone(),
        subject = smtp.subject.clone(),
        boundary = boundary,
    );
    smtp.write(format!("{}{}\r\n", data_header, data_msg).as_bytes())
        .await
        .unwrap();
    log::info!("Email headers and body sent.");
    smtp.write(b"\r\n.\r\n").await.unwrap();
    State::MailSent
}
