use std::process::Command;
use std::time::Duration;

use futures_util::StreamExt;
use linear_rat::terminal::{self, JobControl};

// One test for all signals: every shutdown future in the process sees every
// signal, so separate tests running in parallel would pass on each other's.
#[tokio::test]
async fn interrupt_terminate_and_hangup_each_ask_lrat_to_shut_down() {
    for signal in ["INT", "TERM", "HUP"] {
        let shutdown = terminal::shutdown_signal().unwrap();

        // As `kill -INT <pid>` from a shell would.
        let sent = Command::new("kill")
            .arg(format!("-{signal}"))
            .arg(std::process::id().to_string())
            .status()
            .unwrap();

        assert!(sent.success(), "kill -{signal} failed");
        assert!(
            tokio::time::timeout(Duration::from_secs(2), shutdown)
                .await
                .is_ok(),
            "SIG{signal} did not resolve the shutdown future"
        );
    }
}

#[tokio::test]
async fn tstp_and_cont_come_through_as_job_control_without_stopping_lrat() {
    let mut job_control = Box::pin(terminal::job_control().unwrap());

    for (signal, expected) in [
        ("TSTP", JobControl::Suspend),
        ("CONT", JobControl::Continue),
    ] {
        let sent = Command::new("kill")
            .arg(format!("-{signal}"))
            .arg(std::process::id().to_string())
            .status()
            .unwrap();

        assert!(sent.success(), "kill -{signal} failed");
        let received = tokio::time::timeout(Duration::from_secs(2), job_control.next())
            .await
            .unwrap_or_else(|_| panic!("SIG{signal} did not come through"));
        assert_eq!(received, Some(expected));
    }
}
