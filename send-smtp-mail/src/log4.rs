use log::LevelFilter;
use log4rs::append::console::{self};
use log4rs::config::{Appender, Config, Logger, Root};
use log4rs::encode::pattern::PatternEncoder;
use std::sync::OnceLock;

pub fn init_log() {
    static INIT: OnceLock<()> = OnceLock::new(); // initialise only once
    INIT.get_or_init(|| {
        let logfile = log4rs::append::file::FileAppender::builder()
            .build("my-log.log")
            .unwrap();

        // {d}: Timestamp of the log entry.
        // {l}: Log level (e.g., INFO, DEBUG).
        // {m}: The actual log message.
        // {n}: Newline character.
        // {h}: highlight ??
        let console = console::ConsoleAppender::builder()
            .encoder(Box::new(PatternEncoder::new(
                "{t}:: {h({m}{n})}", // Add custom formatting
            )))
            .build();

        let config = Config::builder()
            .appender(Appender::builder().build("logfile", Box::new(logfile)))
            .appender(Appender::builder().build("console", Box::new(console)))
            .logger(
                Logger::builder()
                    .additive(false)
                    .build("my_module", LevelFilter::Debug), // Set to Debug for detailed logs
            )
            .build(
                Root::builder()
                    .appender("console")
                    .appender("logfile")
                    .build(LevelFilter::Info),
            )
            .unwrap();

        log4rs::init_config(config).unwrap();
    });
}
