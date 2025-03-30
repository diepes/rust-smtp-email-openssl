use base64::engine::general_purpose::STANDARD as b64;
use base64::Engine; // trait
use colored::*;
use dotenv::dotenv;
use std::env;
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
            "Test mail Rust OpenSSL - smtp email sent at {}",
            chrono::Local::now()
        )
    });

    println!(
        "[DEBUG] Connecting to SMTP server: {} -starttls smtp",
        smtp_server
    );

    // Start openssl s_client process
    let mut child = Command::new("openssl")
        .args(["s_client", "-connect", &smtp_server, "-starttls", "smtp"])
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
    wait_for_response_out(&rx_out, "354");

    // Send email content
    let email_body = format!(
        "Test smtp: Rust OpenSSL\n\nFrom={}\nTo={}\nSubject={}\n",
        from, to, subject
    );
    let email = format!(
        "From: {}\r\nTo: {}\r\nSubject: {}\r\n\r\n{}",
        from, to, subject, email_body
    );
    send_cmd(&stdin, &email);
    send_cmd(&stdin, "\r\n."); // send_cmd will add \r\n
    wait_for_response_out(&rx_out, "250");

    send_cmd(&stdin, "QUIT");
    wait_for_response_err(&rx_err, "DONE");

    // Wait for the reader thread to finish
    handle_out.join().unwrap();
    handle_err.join().unwrap();

    println!("{}","[DEBUG] Email sent successfully!".on_green());
}

/// **Spawns a thread to read from OpenSSL's output**
fn spawn_reader_out(stdout: ChildStdout, tx_out: Sender<String>, debug: bool) -> thread::JoinHandle<()> {
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
            println!("{}", "[SMTP StdErr] !!!STARTED StdERR!!!".yellow());
        };
        for line in reader.lines() {
            if let Ok(line) = line {
                if debug {
                    println!("{} {}", "[SMTP StdErr]".yellow(), line.yellow());
                };
                if tx_err.send(line).is_err() {
                    println!("{}", "[SMTP StdErr] !!!EXIT!!!".yellow());
                    break;
                }
            }
        }
    })
}

/// **Sends an SMTP command**
fn send_cmd(mut stdin: &ChildStdin, cmd: &str) {
    //let mut stdin = stdin.clone();
    write!(stdin, "{}\r\n", cmd).expect("Failed to send command");
    println!("{} {}", "[DEBUG stdin] Sent:".blue(), cmd.blue());
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
