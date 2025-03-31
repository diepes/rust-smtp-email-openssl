use base64::engine::general_purpose::STANDARD as b64;
use base64::Engine; // trait
use colored::*;
use dotenv::dotenv;
use std::env;
use std::fs;
//use std::io::{self, Write};
use std::io::{BufRead, BufReader, Write};
use std::process::{ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

// https://learn.microsoft.com/en-us/azure/communication-services/concepts/service-limits

fn main() {
    dotenv().ok();
    let smtp_server = env::var("smtp_server").expect("smtp_server .env not set");
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
    let attachment_path =
        env::var("smtp_attachment_path").unwrap_or_else(|_| format!("example.txt")); // Replace with your file path
    let attachment_data = fs::read(&attachment_path).expect(&format!(
        "Failed to read attachment file {}",
        attachment_path
    ));
    let attachment_encoded = b64.encode(&attachment_data);
    let attachment_name = &attachment_path;

    println!(
        "[DEBUG] Connecting to SMTP server: {} -starttls smtp",
        smtp_server
    );

    // Start openssl s_client process
    let mut child = Command::new("openssl")
        .args([
            "s_client",
            "-connect",
            &smtp_server,
            "-starttls",
            "smtp",
            // "-quiet", // hides "CONNECTED" message
            "-tls1_2",
            // "-no_legacy_server_connect",
            // "-legacy_renegotiation",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to start OpenSSL process");

    let stdin = child.stdin.take().expect("Failed to open stdin");
    let stdout = child.stdout.take().expect("Failed to open stdout");
    let stderr = child.stderr.take().expect("Failed to open stderr");

    let (tx_out, rx_out) = mpsc::channel();
    let (tx_err, rx_err) = mpsc::channel();
    let handle_out = spawn_reader_out(stdout, tx_out, debug);
    let handle_err = spawn_reader_err(stderr, tx_err, debug);

    // SMTP Connected
    wait_for_response_out(&rx_out, "CONNECTED");
    thread::sleep(std::time::Duration::from_secs(1));
    // wait_for_response_err(&rx_err, "250-SIZE");
    wait_for_response_err(&rx_err, "250 STARTTLS");
    // sleep for 1 second
    thread::sleep(std::time::Duration::from_secs(1));
    // Empty rx_out
    while let Ok(_value) = rx_out.try_recv() {}

    // SMTP Conversation
    send_cmd(&stdin, "EHLO rustclient");
    wait_for_response_out(&rx_out, "250 AUTH LOGIN");
    thread::sleep(std::time::Duration::from_secs(1));

    send_cmd(&stdin, &format!("AUTH LOGIN"));
    // Ask for username
    wait_for_response_out(&rx_out, &format!("334 {}", b64.encode("Username:")));
    send_cmd(&stdin, &format!("{}", b64.encode(smtp_username)));
    wait_for_response_out(&rx_out, &format!("334 {}", b64.encode("Password:")));
    send_cmd(&stdin, &format!("{}", b64.encode(smtp_password)));
    wait_for_response_out(&rx_out, "235");

    // 535 5.7.3 Authentication unsuccessful

    send_cmd(&stdin, &format!("mail from: <{}>", from));
    wait_for_response_out(&rx_out, "250");

    send_cmd(&stdin, &format!("rcpt to: <{}>", to));
    wait_for_response_out(&rx_out, "250");

    send_cmd(&stdin, "DATA");
    wait_for_response_out(&rx_out, "354 Start");

    // Chunk the base64 output into lines of 76 characters
    let chunk_size = 990; // RFC 2045 recommends 76 characters per line, RFC 3822 allows 998
    let chunked_encoded: String = attachment_encoded
        .as_bytes()
        .chunks(chunk_size)
        .map(|chunk| {
            std::str::from_utf8(chunk)
                .map_err(|e| {
                    eprintln!("UTF-8 conversion error: {}", e);
                    String::from("INVALID UTF-8") // or return a fallback chunk
                })
                .unwrap_or("INVALID UTF-8") // Fallback if needed
        })
        .collect::<Vec<&str>>()
        .join("\r\n");

    // Send email content

    // MIME multipart message
    let boundary = "boundary123456789";
    // Use `chunked_encoded` instead of `attachment_encoded` in the email body
    let _email_body = format!(
        "From: {from}\r\n\
        To: {to}\r\n\
        Subject: {subject}\r\n\
        MIME-Version: 1.0\r\n\
        Content-Type: multipart/mixed; boundary=\"{boundary}\"\r\n\
        \r\n\
        --{boundary}\r\n\
        Content-Type: text/plain; charset=utf-8\r\n\
        \r\n\
        This is the email body.\r\n\
        \r\n\
        --{boundary}\r\n\
        Content-Type: application/octet-stream\r\n\
        Content-Disposition: attachment; filename={attachment_name}\r\n\
        Content-Transfer-Encoding: base64\r\n\
        \r\n\
        {chunked_encoded}\r\n\
        --{boundary}--\r\n",
        from = from,
        to = to,
        subject = subject,
        boundary = boundary,
        attachment_name = attachment_name,
        chunked_encoded = chunked_encoded
    );

    // let email_body =

    // Send the email
    println!("{}", "[DEBUG] ... Sending email body:".green());
    send_cmd(
        &stdin,
        &format!(
            "From: {from}\r\n\
        To: {to}\r\n\
        Subject: {subject}\r\n\
        MIME-Version: 1.0\r\n\
        Content-Type: multipart/mixed; boundary=\"{boundary}\"\r\n\
        \r\n\
        --{boundary}\r\n\
        Content-Type: text/plain; charset=utf-8\r\n\
        \r\n\
        This is the email body.\r\n\
        \r\n\
        Was sent from {from} to {to}.\r\n\
        \r\n\
        Subject: \"{subject}\"\r\n\
        \r\n\
        See the attached file!\r\n\
        \r\n\
        --{boundary}\r\n\
        Content-Type: application/octet-stream\r\n\
        Content-Disposition: attachment; filename=\"{attachment_name}\"\r\n\
        Content-Transfer-Encoding: base64\r\n\
        \r\n\
        {chunked_encoded}\r\n\
        \r\n\
        --{boundary}--\r\n",
            from = from,
            to = to,
            subject = subject,
            attachment_name = attachment_name,
            chunked_encoded = chunked_encoded,
            boundary = boundary
        ),
    );
    // send_cmd(&stdin, &email_body);

    // sleep for 5 second

    // Send "." dot
    println!(
        "{}",
        "[DEBUG] ... Sending final dot for email body:".green()
    );
    send_cmd(&stdin, "\r\n.");
    wait_for_response_out(&rx_out, "250");

    send_cmd(&stdin, "QUIT");
    wait_for_response_err(&rx_err, "DONE");

    // Wait for the reader thread to finish
    handle_out.join().unwrap();
    handle_err.join().unwrap();

    println!(
        "{} attachment:{} ✅ Size: {}MB",
        "[DEBUG] Email sent successfully!".on_green(),
        attachment_name,
        attachment_encoded.len() / 1024 / 1024
    );
}

/// **Spawns a thread to read from OpenSSL's output**
fn spawn_reader_out(
    stdout: ChildStdout,
    tx_out: Sender<String>,
    debug: bool,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        if debug {
            println!("[SMTP in] !!!STARTED StdOUT!!!");
        };
        for line in reader.lines() {
            if let Ok(line) = line {
                if debug {
                    println!("[SMTP in] {}", line);
                };
                if tx_out.send(line).is_err() {
                    println!("[SMTP in] reader !!!EXIT!!!");
                    break;
                }
            }
        }
    })
}

/// **Spawns a thread to read from OpenSSL's stderr**
fn spawn_reader_err(
    stderr: ChildStderr,
    tx_err: Sender<String>,
    debug: bool,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        if debug {
            println!("{}", "[SMTP StdErr reader] !!!STARTED StdERR!!!".yellow());
        };
        for line in reader.lines() {
            if let Ok(line) = line {
                if debug {
                    println!("{} {}", "[SMTP StdErr reader]".yellow(), line.yellow());
                };
                if tx_err.send(line).is_err() {
                    println!("{}", "[SMTP StdErr reader] !!!EXIT!!!".yellow());
                    thread::sleep(std::time::Duration::from_secs(2));
                    break;
                }
            } else {
                println!(
                    "{} {}",
                    "[SMTP StdErr reader]".yellow(),
                    "!! Nothing to read ???".yellow()
                );
            }
        }
    })
}

/// **Sends an SMTP command**
fn send_cmd(mut stdin: &ChildStdin, cmd: &str) {
    //let mut stdin = stdin.clone();
    print!("{}", "[DEBUG send] Sending command:".blue());
    write!(stdin, "{}\r\n", cmd).expect("Failed to send command");
    println!("{} {}", "[DEBUG send] Sent:".blue(), cmd.blue());
}

/// **Waits for a specific response from the SMTP server**
fn wait_for_response_out(rx_out: &Receiver<String>, expected: &str) {
    println!(
        "{} {}",
        "[DEBUG wait StdOut] Waiting for:".green(),
        expected.green()
    );
    while let Ok(line) = rx_out.recv() {
        if line.starts_with(expected) {
            println!(
                "{} {}",
                "[DEBUG wait StdOut] matched:".green(),
                line.green()
            );
            break;
        }
        println!(
            "{} {}",
            "[DEBUG wait StdOut] ... received noise:".green(),
            line.green()
        );
    }
}
/// **Waits for a specific response from the SMTP server**
fn wait_for_response_err(rx_err: &Receiver<String>, expected: &str) {
    println!(
        "{} {}",
        "[DEBUG wait StdErr] Waiting for:".red(),
        expected.red()
    );
    while let Ok(line) = rx_err.recv() {
        if line.starts_with(expected) {
            println!("{} {}", "[DEBUG wait StdErr] matched:".red(), line.red());
            break;
        }
        println!(
            "{} {}",
            "[DEBUG wait StdErr] ... received noise:".red(),
            line.red()
        );
    }
}
