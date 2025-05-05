use crate::state_machine::State;
use crate::stream::SmtpConnection; // Import State from the appropriate module

use base64::engine::general_purpose::STANDARD as b64;
use base64::Engine; // trait

fn chunk_and_encode(data: &Vec<u8>, chunk_size: usize) -> Vec<Vec<u8>> {
    // Base64 encode the entire data
    let encoded = b64.encode(data);

    // Handle chunk_size == 0: return all data in one entry with \r\n
    if chunk_size == 0 {
        let chunk = encoded.into_bytes();
        // chunk.extend_from_slice(b"\r\n");
        return vec![chunk];
    }

    // Reduce chunk_size to the nearest lower multiple of 4
    let adjusted_chunk_size = (chunk_size / 4) * 4;

    // If adjusted_chunk_size is 0, use 4 as the minimum
    let final_chunk_size = if adjusted_chunk_size == 0 {
        4
    } else {
        adjusted_chunk_size
    };

    // Chunk the base64-encoded bytes and append \r\n to each
    encoded
        .as_bytes()
        .chunks(final_chunk_size)
        .map(|chunk| {
            let mut chunk_vec = chunk.to_vec();
            chunk_vec.extend_from_slice(b"\r\n");
            chunk_vec
        })
        .collect()
}

pub async fn send_body(smtp: &mut SmtpConnection) -> State {
    // log4::init_log();
    // Send the email body
    log::info!("Sending email body...");
    let boundary = "boundary123456789";
    let attachment_data_b64 = chunk_and_encode(
        &smtp.attachement_data.as_ref().unwrap_or(&Vec::new()),
        0, // 1MB chunks
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
        \r\n",
        from = smtp.from.clone(),
        to = smtp.to.clone(),
        subject = smtp.subject.clone(),
    );
    smtp.write(format!("{}{}\r\n", data_header, data_msg).as_bytes())
        .await
        .unwrap();
    log::info!("Email headers and body sent.");

    if attachment_data_b64.is_empty() || smtp.attachement_name.is_none() {
        log::info!("No attachment data to send.");
        smtp.write(format!("\r\n--{boundary}--\r\n").as_bytes())
        .await
        .unwrap();
        smtp.write(b"\r\n.\r\n").await.unwrap();
        return State::MailSent;
    }
    // Send the attachment
    log::info!("Sending attachment... {}", smtp.attachement_name.clone().unwrap_or_default());
    let start_send = std::time::Instant::now();
    let mut send_size = 0;
    smtp.write(
        format!(
            "--{boundary}\r\n\
            Content-Type: application/octet-stream\r\n\
            Content-Disposition: attachment; filename=\"{}\"\r\n\
            Content-Transfer-Encoding: base64\r\n",
            smtp.attachement_name.clone().unwrap_or_default()
        )
        .as_bytes(),
    )
    .await
    .unwrap();
    for (i, chunk) in attachment_data_b64.iter().enumerate() {
        smtp.write(&chunk).await.unwrap();
        send_size += chunk.len();
        if send_size % (1024 * 1024) == 0 {
            log::info!(
                "Attachment chunk {} sent. size:{} bytes {:.2} Mb",
                i,
                send_size,
                send_size as f64 / (1024.0 * 1024.0)
            );
        }
    }
    smtp.flush().await.unwrap();
    log::info!(
        "Attachment sent. size:{}b = {:.2}Mb in {:.2}sec",
        send_size,
        send_size as f64 / (1024.0 * 1024.0),
        start_send.elapsed().as_secs_f64()
    );
    smtp.write(format!("\r\n--{boundary}--\r\n\r\n").as_bytes())
        .await
        .unwrap();
    smtp.flush().await.unwrap();
    smtp.write("\r\n.\r\n".as_bytes()).await.unwrap();
    smtp.flush().await.unwrap();
    log::info!("Final boundary and dot . sent.");
    State::MailSent
}
