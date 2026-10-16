use memreduct_rs::{
    config::{append_report, config_path, Config},
    model::{gib, Profile},
    platform, AppResult,
};

const HELP: &str = "MemReduct RS CLI 0.1.0\n\
Usage:\n  memreduct-cli status [--json]\n  memreduct-cli clean [--profile conservative|balanced|aggressive] [--json] [--yes] [--portable]\n  memreduct-cli config [--portable]\n\n\
Status needs no elevation. Clean requires an elevated terminal.\n\
Aggressive mode requires --yes and can cause disk IO and temporary stalls.\n\
--portable stores config/log beside the executable. Exit 0=success, 1=error, 2=invalid arguments, 3=partial/failed cleanup.\n";

fn main() {
    match run(std::env::args().skip(1).collect()) {
        Ok(code) => std::process::exit(code),
        Err((code, message)) => {
            eprintln!("{message}");
            std::process::exit(code);
        }
    }
}

fn run(args: Vec<String>) -> Result<i32, (i32, String)> {
    if args.is_empty() || args == ["--help"] || args == ["-h"] {
        print!("{HELP}");
        return Ok(0);
    }
    let command = &args[0];
    if !["status", "clean", "config"].contains(&command.as_str()) {
        return Err((2, HELP.into()));
    }
    let mut profile = Profile::Conservative;
    let mut json = false;
    let mut yes = false;
    let mut portable = false;
    let mut profile_set = false;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--json" if command != "config" && !json => json = true,
            "--portable" if command != "status" && !portable => portable = true,
            "--yes" if command == "clean" && !yes => yes = true,
            "--profile" if command == "clean" && !profile_set => {
                i += 1;
                profile = Profile::parse(args.get(i).ok_or((2, "missing --profile value".into()))?)
                    .map_err(|e| (2, e))?;
                profile_set = true;
            }
            value => return Err((2, format!("Invalid/duplicate option: {value}\n{HELP}"))),
        }
        i += 1;
    }
    let result: AppResult<i32> = (|| match command.as_str() {
        "status" => {
            let m = platform::snapshot()?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&m).map_err(|e| e.to_string())?
                );
            } else {
                println!(
                    "物理内存 {}%，可用 {} / {}\n提交量 {} / {}，系统缓存 {}",
                    m.load_percent,
                    gib(m.physical_available),
                    gib(m.physical_total),
                    gib(m.commit_total),
                    gib(m.commit_limit),
                    gib(m.system_cache)
                );
            }
            Ok(0)
        }
        "config" => {
            let path = config_path(portable)?;
            let c = Config::load_or_create(&path)?;
            println!(
                "{}\n{}",
                path.display(),
                toml::to_string_pretty(&c).map_err(|e| e.to_string())?
            );
            Ok(0)
        }
        "clean" => {
            if profile == Profile::Aggressive && !yes {
                return Err("强力清理可能引起卡顿和磁盘写入，请添加 --yes 明确执行".into());
            }
            let path = config_path(portable)?;
            Config::load_or_create(&path)?;
            let report = platform::clean(profile)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
                );
            } else {
                println!("{}", report.summary());
                for op in &report.operations {
                    println!(
                        "{}: {}",
                        op.operation.label(),
                        op.error.as_deref().unwrap_or("OK")
                    );
                }
                if let Some(error) = &report.measurement_error {
                    eprintln!("{error}");
                }
            }
            if let Err(error) = append_report(&path, &report) {
                eprintln!("日志写入失败：{error}");
            }
            Ok(
                if report.succeeded() && report.measurement_error.is_none() {
                    0
                } else {
                    3
                },
            )
        }
        _ => unreachable!(),
    })();
    result.map_err(|e| (1, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_ambiguous_or_misplaced_options_before_any_system_call() {
        for args in [
            vec!["status", "--profile", "aggressive"],
            vec!["clean", "--profile"],
            vec!["clean", "--yes", "--yes"],
            vec!["config", "--json"],
            vec!["clean", "--profile", "typo"],
        ] {
            assert_eq!(
                run(args.iter().map(|s| s.to_string()).collect())
                    .unwrap_err()
                    .0,
                2
            );
        }
    }
}
