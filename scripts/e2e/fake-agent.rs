use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;
use std::{env, fs, thread};

fn out(s: &str) {
    println!("{s}");
    std::io::stdout().flush().unwrap();
}

fn success(text: &str) {
    out(&format!(
        r#"{{"event":"result","result":{{"conversation_id":"conv-1","status":"SUCCESS","response":"{text}"}}}}"#
    ));
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.get(1).map(String::as_str) == Some("--sleep-child") {
        thread::sleep(Duration::from_secs(300));
        return;
    }
    let prompt = args.last().cloned().unwrap_or_default();
    let cmd = prompt.split_whitespace().next().unwrap_or("");
    out(r#"{"event":"init","conversation_id":"conv-1"}"#);
    match cmd {
        "FAIL" => std::process::exit(3),
        "ERR" => out(
            r#"{"event":"result","result":{"conversation_id":"conv-1","status":"ERROR","response":"model exploded"}}"#,
        ),
        "SLEEP" => {
            let child = Command::new(env::current_exe().unwrap())
                .arg("--sleep-child")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            if let Ok(path) = env::var("FAKE_PID_FILE") {
                fs::write(path, format!("{} {}", std::process::id(), child.id())).unwrap();
            }
            thread::sleep(Duration::from_secs(300));
        }
        "SLOWOK" => {
            thread::sleep(Duration::from_secs(6));
            success("done-slow");
        }
        _ => success("done"),
    }
}
