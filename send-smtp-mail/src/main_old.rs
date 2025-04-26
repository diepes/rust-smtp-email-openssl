use base64::engine::general_purpose::STANDARD as b64;
use base64::Engine; // trait
use colored::*;
use dotenv::dotenv;
use std::env;
use std::fs;
use std::io;
use std::io::{BufRead, BufReader, Write};
use std::process::{exit, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::vec;

// use local module smtp_starttls::smtp_starttls;
mod smtp_starttls;

// https://learn.microsoft.com/en-us/azure/communication-services/concepts/service-limits

#[tokio::main]
async fn main() {
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
            // "-tls1_2",
            // "-no_legacy_server_connect",
            "-ign_eof", // Prevents OpenSSL from closing on unexpected EOF, e.g. ssl renegotiation
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
    send_cmd(&stdin, debug, "EHLO rustclient");
    wait_for_response_out(&rx_out, "250 AUTH LOGIN");
    thread::sleep(std::time::Duration::from_secs(1));

    send_cmd(&stdin, debug, &format!("AUTH LOGIN"));
    // Ask for username
    wait_for_response_out(&rx_out, &format!("334 {}", b64.encode("Username:")));
    send_cmd(&stdin, debug, &format!("{}", b64.encode(&smtp_username)));
    wait_for_response_out(&rx_out, &format!("334 {}", b64.encode("Password:")));
    send_cmd(&stdin, debug, &format!("{}", b64.encode(smtp_password)));
    wait_for_response_out(&rx_out, "235");

    // 535 5.7.3 Authentication unsuccessful

    send_cmd(&stdin, debug, &format!("mail from: <{}>", from));
    wait_for_response_out(&rx_out, "250");

    send_cmd(&stdin, debug, &format!("rcpt to: <{}>", to));
    wait_for_response_out(&rx_out, "250");

    send_cmd(&stdin, debug, "DATA");
    wait_for_response_out(&rx_out, "354 Start");

    // MIME multipart message
    let boundary = "boundary123456789";
    // Use `chunked_encoded` instead of `attachment_encoded` in the email body

    println!("{}", "[DEBUG] ... Sending email headers ...".purple());
    send_cmd(
        &stdin,
        debug,
        &format!(
            "From: {from}\r\n\
            To: {to}\r\n\
            Subject: {subject}"
        ),
    );
    println!("{}", "[DEBUG] ... Sending email mime-version ...".purple());
    send_cmd(
        &stdin,
        debug,
        &format!(
            "MIME-Version: 1.0\r\n\
            Content-Type: multipart/mixed; boundary=\"{boundary}\"\r\n\
            \r\n\
            --{boundary}\r\n\
            Content-Type: text/plain; charset=utf-8\r\n"
        ),
    );

    // Chunk the base64 output into lines of 76 characters, Set to 1MB for large files 20MB 28 chunks.
    // let chunk_size = 1024; //2048; // RFC 2045 recommends 76 characters per line, RFC 3822 allows 998
    let chunk_size = 2048000; // 204kB_crash_sec, 102kB_crash_98sec
    let chunked_encoded = attachment_encoded
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
        .collect::<Vec<&str>>();

    println!("{}", "[DEBUG] ... Sending email txt body ...".purple());
    send_cmd(
        &stdin,
        debug,
        &format!(
            "This is the email body.\r\n\
            \r\n\
            Was sent from {from} to {to}.\r\n\
            \r\n\
            Subject: \"{subject}\"\r\n\
            \r\n\
            See the attached file!\r\n\
            \r\n\
            SETTINGS:\r\n\
            SMTP_SERVER: {smtp_server}\r\n\
            SMTP_USERNAME: {smtp_username}\r\n\
            SMTP_ATTACHMENT_NAME: {attachment_name}\r\n\
            SMTP_ATTACHMENT_SIZE: {size} bytes\r\n\
            SMTP_ATTACHMENT_SIZE: {size_mb} MB\r\n\
            SMTP_ATTACHMENT_B64_SIZE_MB: {size_mb_b64} MB\r\n\
            SMTP_ATTACHMENT_CHUNK_SIZE: {chunk_size} bytes\r\n\
            \r\n",
            size = attachment_data.len(),
            size_mb = attachment_data.len() / 1024 / 1024,
            size_mb_b64 = attachment_encoded.len() / 1024 / 1024,
        ),
    );

    if attachment_data.len() > 0 {
        // Create email mime attachment
        send_cmd(
            &stdin,
            debug,
            &format!(
                "--{boundary}\r\n\
            Content-Type: application/octet-stream\r\n\
            Content-Disposition: attachment; filename=\"{attachment_name}\"\r\n\
            Content-Transfer-Encoding: base64\r\n"
            ),
        );

        // Send email content
        println!(
        "{}\n[{}\n...\n{}]\n{}",
        "[DEBUG] ... Sending email attachement begining ... end".purple(),
        chunked_encoded[1][0..120].purple(),
        chunked_encoded[chunked_encoded.len() - 1].chars().rev().take(120).collect::<Vec<_>>().into_iter().rev().collect::<String>().purple(),
        format!(
            "Length:{}/{}/{}x{} modulo3:{} Note: mod3=0 no padding,  mod3=1 two (==),  mod3=2 one (=)",
            attachment_data.len(),
            attachment_encoded.len(),
            chunked_encoded.len(),
            chunk_size,
            attachment_data.len() % 3,
        )
        .purple()
    );
        // Send the attachment in chunks in for loop
        println!(
            "{} {}",
            "[DEBUG] ... Sending email attachment lines:".green(),
            chunked_encoded.len()
        );
        let start_send = std::time::Instant::now();
        let num_lines = chunked_encoded.len();
        let mut send_size = 0;
        for (i, &attachment_b64_line) in chunked_encoded.iter().enumerate() {
            if debug {
                println!(
                    "{} {}",
                    "[DEBUG] ... Sending email attachment line:".purple(),
                    i + 1
                );
            };
            let err_msg = send_cmd_capture_err(&stdin, debug, attachment_b64_line);
            send_size += attachment_b64_line.len();
            print!(
                "{}/{num_lines} {:.2}s {}b {}   \r",
                i + 1,
                start_send.elapsed().as_secs_f64(),
                send_size,
                err_msg.map_or_else(|e| format!("ERR: {:?}", e).red(), |_| "OK".green(),),
            );
            io::stdout().flush().unwrap();
            // short sleep to avoid overwhelming the server
            // thread::sleep(std::time::Duration::from_millis(50));
            if let Ok(err_msg) = rx_err.try_recv() {
                println!(
                    "{} {} {}",
                    "[DEBUG] ... Got error msg:".purple(),
                    err_msg,
                    "sleep for 5s"
                );
                thread::sleep(std::time::Duration::from_secs(5));
            }
        }
        println!(); // Print a new line after the progress bar
                    // send_cmd(&stdin, &chunked_encoded);
    }; // end if attachment_path

    println!("{}", "[DEBUG] ... Sending final mime boundary ...".purple());
    send_cmd(
        &stdin,
        debug,
        &format!(
            "\r\n--{boundary}--\r\n\
        \r\n"
        ),
    );
    // Send "." dot
    println!("{}", "[DEBUG] ... Sending dot . to end email ...".purple());
    send_cmd(&stdin, debug, "\r\n.");

    if let Some((i, m, st)) = try_for_response_out(&rx_out, ["250", "501"], 5) {
        if i == 0 {
            println!("{} {}", "[DEBUG] ... Received :".green(), m.green().bold());
        } else if i == 1 {
            println!(
                "{} {}",
                "[DEBUG] ... Received Error:".red(),
                st.red().bold()
            );
            // check if any error message
            try_for_response_out(&rx_err, [], 2);
            try_for_response_out(&rx_out, [], 2);
            exit(1);
        }
    };

    // check if any error message
    try_for_response_out(&rx_err, [], 2);

    println!(
        "{} {}",
        "[DEBUG] ... Sending QUIT, and waiting for DONE ...".purple(),
        "QUIT".purple()
    );

    send_cmd(&stdin, debug, "QUIT");

    // check if any error message
    try_for_response_out(&rx_err, ["DONE"], 2);
    try_for_response_out(&rx_out, ["221"], 2);
    // Wait for the reader thread to finish
    handle_out.join().unwrap();
    handle_err.join().unwrap();

    println!(
        "{} attachment:{} ✅ b64 Size: {}MB",
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
                    println!("{} {}", "[SMTP StdErr reader] <<".yellow(), line.yellow());
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
fn send_cmd(mut stdin: &ChildStdin, debug: bool, cmd: &str) {
    //let mut stdin = stdin.clone();
    // print!("{}", "[DEBUG send] Sending command:".blue());
    if debug {
        println!("{} {}", "[DEBUG send]".blue(), cmd.blue());
    };
    write!(stdin, "{}\r\n", cmd).expect("[DEBUG send] Failed to send command");
    stdin.flush().expect("[DEBUG send] Failed to flush stdin"); // <-- Ensures data is sent immediately
}
/// **Sends an SMTP command, returns errors**
fn send_cmd_capture_err(
    mut stdin: &ChildStdin,
    debug: bool,
    cmd: &str,
) -> Result<(), std::io::Error> {
    //let mut stdin = stdin.clone();
    // print!("{}", "[DEBUG send] Sending command:".blue());
    if debug {
        println!("{} {}", "[DEBUG send]".blue(), cmd.blue());
    };
    write!(stdin, "{}\r\n", cmd)?;
    stdin.flush().expect("[DEBUG send] Failed to flush stdin"); // <-- Ensures data is sent immediately
    Ok(())
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
/// **Waits for a specific response from the SMTP server with timeout**
/// * Returns the index of the matched string, the matched string, and the line
fn try_for_response_out<'a, T: IntoIterator<Item = &'a str>>(
    rx_out: &'a Receiver<String>,
    expected: T,
    mut timeout_sec: i32,
) -> Option<(usize, &'a str, String)> {
    let expected_strings = expected.into_iter().collect::<Vec<&str>>();
    println!(
        "{} {}",
        "[DEBUG try StdOut] Waiting for:".green(),
        expected_strings.join(" or ").green()
    );
    while timeout_sec > 0 {
        if let Ok(line) = rx_out.try_recv() {
            if let Some(exp_found) = expected_strings
                .iter()
                .enumerate()
                .find(|(_i, &exp)| line.starts_with(exp))
            {
                println!(
                    "{} {} [{}]",
                    "[DEBUG try StdOut] matched:".green(),
                    line.green(),
                    exp_found.1.blue()
                );
                return Some((exp_found.0, exp_found.1, line));
            } else {
                println!(
                    "{} {}",
                    "[DEBUG try StdOut] ... received noise:".green(),
                    line.green()
                );
            }
        }
        if timeout_sec >= 0 {
            timeout_sec -= 1;
            thread::sleep(std::time::Duration::from_secs(1));
        }
    }
    None
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
            return;
        }
        println!(
            "{} {}",
            "[DEBUG wait StdErr] ... received noise:".red(),
            line.red()
        );
    }
    println!(
        "{} {}",
        "[DEBUG wait StdErr] ... ERR while waiting for".red(),
        expected.red()
    );
}
