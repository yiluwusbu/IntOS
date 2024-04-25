use std::env;
use toml::Value;

type Table = toml::value::Map<String, Value>;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=.cargo/config.toml");
    println!("cargo:rerun-if-env-changed=ENV_BOARD");
    println!("cargo:rerun-if-env-changed=ENV_BENCH_TASK");
    println!("cargo:rerun-if-env-changed=ENV_BASELINE");
    println!("cargo:rerun-if-env-changed=ENV_SRAM_BASELINE");
    println!("cargo:rerun-if-env-changed=ENV_TIMER_DAEMON");
    println!("cargo:rerun-if-env-changed=ENV_IDEM");
    println!("cargo:rerun-if-env-changed=ENV_PF_FREQ");
    println!("cargo:rerun-if-env-changed=ENV_APP");
    let env_board = env::var("ENV_BOARD").unwrap().to_lowercase();
    let bench_task = match env::var("ENV_BENCH_TASK") {
        Ok(v) => v.to_lowercase(),
        Err(_) => "".to_string(),
    };
    let idem = match env::var("ENV_IDEM") {
        Ok(_) => true,
        Err(_) => false,
    };

    let baseline = match env::var("ENV_BASELINE") {
        Ok(_) => true,
        Err(_) => false,
    };

    let sram_baseline = match env::var("ENV_SRAM_BASELINE") {
        Ok(_) => true,
        Err(_) => false,
    };

    let user_app = match env::var("ENV_APP") {
        Ok(v) => v,
        Err(_) => "".to_string(),
    };
    if !user_app.is_empty() {
        println!("cargo:rustc-cfg=app=\"{}\"", user_app);
        println!("cargo:warning=User app set to  {}", user_app);
    }
    assert!(!(sram_baseline && baseline));

    let timer_daemon = match env::var("ENV_TIMER_DAEMON") {
        Ok(_) => true,
        Err(_) => false,
    };

    let pf_freq = match env::var("ENV_PF_FREQ") {
        Ok(v) => v.to_lowercase(),
        Err(_) => "".to_string(),
    };

    let mut ld_script_path: String = "link_scripts/link_".to_owned();
    let board_flag;

    match env_board.as_str() {
        "qemu" => {
            ld_script_path.push_str("qemu");
            board_flag = "qemu";
            println!("cargo:rustc-cfg=armv7em");
        }
        "apollo4bp" => {
            ld_script_path.push_str("apollo");
            board_flag = "apollo4bp";
            println!("cargo:rustc-cfg=armv7em");
        }
        "msp430fr5994" => {
            ld_script_path.push_str("msp430");
            board_flag = "msp430fr5994";
        }
        "test" => {
            ld_script_path.push_str("test");
            board_flag = "test";
        }
        "" => {
            panic!("Please specify ENV_BOARD (the name of board to install the os onto)")
        }
        _ => {
            panic!("Unsupported Board")
        }
    }
    if idem {
        if board_flag != "qemu" {
            ld_script_path.push_str("_nvm.x");
        } else {
            ld_script_path.push_str(".x");
        }
        println!("cargo:rustc-cfg=idem");
        // set config flags
    } else if baseline {
        if board_flag != "qemu" {
            ld_script_path.push_str("_nvm.x");
        } else {
            ld_script_path.push_str(".x");
        }
        println!("cargo:rustc-cfg=baseline");
    } else if sram_baseline {
        if board_flag != "qemu" {
            ld_script_path.push_str("_sram.x");
        } else {
            ld_script_path.push_str(".x");
        }
        println!("cargo:rustc-cfg=sram_baseline");
    } else {
        ld_script_path.push_str(".x");
    }
    println!("cargo:warning=building os for {}, running benchmark task: [{}], idempotent-process = {}, using link script: {}", &env_board, &bench_task, idem, ld_script_path);
    if timer_daemon {
        println!("cargo:rustc-cfg=timer_daemon");
    }

    println!("cargo:rerun-if-changed={}", ld_script_path);
    let link_arg = format!("{}{}", "-T", ld_script_path);
    if board_flag != "test" {
        println!("cargo:rustc-link-arg={}", link_arg);
    }
    println!("cargo:rustc-cfg=board=\"{}\"", board_flag);
    println!("cargo:rustc-cfg=bench_task=\"{}\"", bench_task);
    println!("cargo:rustc-cfg=pf_freq=\"{}\"", pf_freq);

    // Generate config file to be included if config file exists
    generate_config_files(board_flag, &bench_task);
}

#[derive(Clone, Copy, Debug)]
struct KernelParams {
    pm_heap_size_per_task: Option<u32>,
    boot_pm_heap_size: Option<u32>,
    pm_journal_size: Option<u32>,
    task_num_limit: Option<u32>,
    pm_heap_size: Option<u32>,
}

fn check_features(f: &str) -> bool {
    let f = f.to_uppercase();
    let f_env_name = "CARGO_FEATURE_".to_owned() + f.as_str();
    return env::var(f_env_name).is_ok();
}

const KERNEL_PARAM_NAMES: [&str; 5] = [
    "pm_heap_size_per_task",
    "boot_pm_heap_size",
    "pm_journal_size",
    "task_num_limit",
    "pm_heap_size",
];

fn is_kernel_params(p: &str) -> bool {
    match p {
        "pm_heap_size_per_task" => true,
        "boot_pm_heap_size" => true,
        "pm_journal_size" => true,
        "task_num_limit" => true,
        "pm_heap_size" => true,
        _ => false,
    }
}

fn get_kernel_params<'a>(p: &str, kparams: &'a mut KernelParams) -> &'a mut Option<u32> {
    match p {
        "pm_heap_size_per_task" => &mut kparams.pm_heap_size_per_task,
        "boot_pm_heap_size" => &mut kparams.boot_pm_heap_size,
        "pm_journal_size" => &mut kparams.pm_journal_size,
        "task_num_limit" => &mut kparams.task_num_limit,
        "pm_heap_size" => &mut kparams.pm_heap_size,
        _ => {
            panic!("Unexpected Parameter {}", p);
        }
    }
}

fn set_kernel_params(p: &str, kparams: &mut KernelParams, v: u32) {
    *get_kernel_params(p, kparams) = Some(v);
}

fn set_config(tbl: &Table, c: &mut KernelParams) {
    // println!("Config is {:?}", tbl);
    for (k, v) in tbl {
        if is_kernel_params(k) {
            let v = v
                .as_integer()
                .unwrap_or_else(|| panic!("Paramter must be define as an integer"));
            set_kernel_params(k, c, v as u32);
        }
    }
}

fn find_longest_matching_features(tbl: &Table) -> Option<&Table> {
    let mut max_matched_features = 0;
    let mut res = None;
    for (k, v) in tbl {
        if is_kernel_params(k) {
            continue;
        }
        let mut ok = true;
        let mut cnt = 0;
        for f in k.split_whitespace() {
            if !check_features(f) {
                ok = false;
                break;
            }
            cnt += 1;
        }
        if ok && cnt > max_matched_features {
            res = v.as_table();
            max_matched_features = cnt;
        }
    }

    return res;
}

fn parse_config(conf_file: &str, bench_name: &str) -> KernelParams {
    let mut ret = KernelParams {
        pm_heap_size_per_task: None,
        boot_pm_heap_size: None,
        pm_journal_size: None,
        task_num_limit: None,
        pm_heap_size: None,
    };

    let tbl = toml::from_str::<Table>(conf_file).unwrap();

    // set values to default
    let default_tbl = tbl
        .get("default")
        .unwrap_or_else(|| panic!("Default parameters must be defined!"))
        .as_table()
        .unwrap_or_else(|| panic!("Default Section value must be a table"));

    set_config(default_tbl, &mut ret);

    for (k_bench, v_sub_tbl) in tbl.into_iter() {
        if k_bench != bench_name || k_bench == "default" {
            continue;
        }
        // benchmark name matched, try matching features
        let v_sub_tbl = match v_sub_tbl.as_table() {
            None => {
                panic!("Must define parameters")
            }
            Some(v) => v,
        };
        let param_tbl = find_longest_matching_features(v_sub_tbl);
        if param_tbl.is_none() {
            set_config(&v_sub_tbl, &mut ret);
        } else {
            let param_tbl = param_tbl.unwrap();
            set_config(param_tbl, &mut ret);
        }
    }

    return ret;
}

fn generate_config_files(board: &str, bench_name: &str) {
    let config_fpath = "./src/board/".to_owned() + board + "/kparam_config.toml";
    let config_consts_fpath = "./src/board/".to_owned() + board + "/kparam_config_constants.inc";
    let config_content = match std::fs::read_to_string(&config_fpath) {
        Ok(v) => v,
        Err(_) => {
            println!(
                "cargo:warning=No kernel parameter config file exists for {}, proceed as normal",
                board
            );
            return;
        }
    };
    println!("cargo:rustc-cfg=kparam_config");
    println!("cargo:rerun-if-changed={}", &config_fpath);
    println!(
        "cargo:warning=using kernel parameter config file {}, generating config for task {}",
        &config_fpath, bench_name
    );

    let mut kparams = parse_config(&config_content, bench_name);
    let mut constants = "// This file is generated by build.rs\n".to_owned();
    for pname in KERNEL_PARAM_NAMES {
        let v = *get_kernel_params(pname, &mut kparams);
        let constant_name = board.to_uppercase() + "_" + pname.to_uppercase().as_str();
        if !v.is_none() {
            let v32 = v.unwrap();
            constants += format!("const {} : usize = {};\n", &constant_name, v32).as_str();
            println!(
                "cargo:warning=Setting constant {} to {}",
                &constant_name, v32
            );
        }
    }
    std::fs::write(&config_consts_fpath, constants)
        .unwrap_or_else(|_| panic!("Failed to create constant file"));
    println!(
        "cargo:warning=kernel parameter config file generated to: {}",
        &config_consts_fpath
    );
}
