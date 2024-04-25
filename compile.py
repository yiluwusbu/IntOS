#!/usr/bin/env python3

import os
import subprocess
import argparse


def main():
    parser = argparse.ArgumentParser(
        description="Compile OS with different configurations"
    )
    parser.add_argument("--dryrun", action="store_true")
    parser.add_argument(
        "--debug",
        action="store_true",
        help="Build the binary in debug mode, which has no compiler optimization and prints out debugging messages",
    )
    parser.add_argument(
        "--board",
        choices=["apollo4bp", "msp430fr5994", "qemu"],
        default="msp430fr5994",
        help="Target development board",
    )
    parser.add_argument(
        "--bench",
        choices=[
            "kv",
            "sense",
            "em",
            "mq",
            "dnn",
            "bc",
            "ar",
            "kv_base",
            "sense_base",
            "em_base",
            "mq_base",
            "dnn_base",
            "bc_base",
            "ar_base",
            "etl",
            "pred",
            "stats",
            "train",
            "none",
        ],
        default="none",
        help="Benchmark application selection, []_base is the benchmark application without undo logging optimization",
    )
    parser.add_argument(
        "--pf_freq",
        choices=["100ns", "200ns", "500ns", "1ms", "2ms", "5ms", "10ms", "none"],
        default="none",
        help="Power failure injection frequency. Must be used together with --fail option",
    )
    parser.add_argument(
        "--disable_list_opt",
        action="store_true",
        help="Disable the list optimization",
    )
    parser.add_argument(
        "--fail",
        dest="features",
        action="append_const",
        const="power_failure",
        help="Inject power failure at a regular interval",
    )
    parser.add_argument(
        "--vanilla",
        action="store_true",
        help="Remove all crash saftey methods employed by the kernel, If this is true, the --disable_list_opt must be set to true",
    )
    parser.add_argument(
        "--no_os_verbose",
        action="store_true",
        help="Disable the display of some resource allocation releated OS messages",
    )
    parser.add_argument(
        "--profile_tx",
        dest="features",
        action="append_const",
        const="profile_tx",
        help="Profiling transactions",
    )
    parser.add_argument(
        "--profile_log",
        dest="features",
        action="append_const",
        const="profile_log",
        help="Profiling log",
    )
    parser.add_argument(
        "--debug_pf",
        dest="features",
        action="append_const",
        const="debug_power_failure",
        help="Print debugging message for power failure",
    )
    parser.add_argument(
        "--debug_bench",
        dest="features",
        action="append_const",
        const="debug_bench",
        help="Print debugging message for benchmark applications",
    )
    parser.add_argument(
        "--use_timerb",
        dest="features",
        action="append_const",
        const="msp430_use_timerb",
        help="Use the timer B in the MSP430FR5994 board",
    )
    parser.add_argument(
        "--bench_kv_large_tx",
        action="store_true",
        help="Use bigger tx size in the KV benchmark",
    )
    parser.add_argument(
        "--riotbench_base",
        dest="features",
        action="append_const",
        const="riotbench_no_log_opt",
        help="Remove log optimization in the RIOT benchmark",
    )
    parser.add_argument(
        "--size_opt",
        action="store_true",
        help="Perform size optimization",
    )

    parser.add_argument(
        "--idem",
        action="store_true",
        help="for idempotent processing. Emit bitcode file and use NVM only",
    )
    parser.add_argument("--base", action="store_true", help="NVM baseline")
    parser.add_argument("--sram_base", action="store_true", help="SRAM baseline")
    parser.add_argument(
        "--run",
        action="store_true",
        help="run applications after compilation (for QEMU)",
    )
    parser.add_argument(
        "--timer_daemon", action="store_true", help="enable timer daemon"
    )
    parser.add_argument(
        "--app",
        help="Specify the name of the user application",
    )
    args = parser.parse_args()
    features = ""

    if args.features is not None:
        features = " ".join(args.features)

    if args.vanilla == False:
        features += " crash_safe"
    
    if args.disable_list_opt == False:
        features += " opt_list"
    
    if args.bench_kv_large_tx == False:
        features += " bench_kv_smaller_tx_sz"
    
    envs_list = []

    if args.bench != "none":
        envs_list.append("ENV_BENCH_TASK=" + args.bench)

    if args.pf_freq != "none":
        envs_list.append("ENV_PF_FREQ=" + args.pf_freq)
    
    if args.app is not None:
        envs_list.append("ENV_APP=" + args.app)

    envs_list.append("ENV_BOARD=" + args.board)

    target = ""
    if args.board == "msp430fr5994":
        target = "msp430-none-elf"
    else:
        target = "thumbv7em-none-eabihf"

    if args.idem:
        envs_list.append("ENV_IDEM=1")
        envs_list.append('RUSTFLAGS="--emit=llvm-bc"')

    if args.base:
        envs_list.append("ENV_BASELINE=1")

    if args.sram_base:
        envs_list.append("ENV_SRAM_BASELINE=1")

    if args.timer_daemon:
        envs_list.append("ENV_TIMER_DAEMON=1")

    build_opt = []

    if args.idem or args.board == "msp430fr5994":
        build_opt = ["-Z", "build-std=core"]

    mode = "build"
    if args.run:
        mode = "run"
    cargo_cmd_list = ["cargo", mode] + build_opt + ["--target", target]

    if args.debug is False:
        if args.size_opt:
            cargo_cmd_list.append("--profile=release-opt-size")
        else:
            cargo_cmd_list.append("--release")

    cargo_cmd_list.append("--no-default-features")

    if "crash_safe" in features:
        if args.bench == "train" and "power_failure" in features and args.board=="apollo4bp": # A hacky fix for a bug revealed in train on apollo4bp during fault injection
            print("removed feature opt_tx_cache_space")
            features += " opt_loop_end"
        else:
            features += " opt_loop_end opt_tx_cache_space"

    if "crash_safe" not in features:
        assert("opt_list" not in features)

    if not args.no_os_verbose:
        features += " verbose_os_info"

    if len(features) != 0:
        cargo_cmd_list.append("--features")
        cargo_cmd_list.append('"' + features + '"')
    

    build_cmd_list = envs_list + cargo_cmd_list
    build_cmd = " ".join(build_cmd_list)
    print(build_cmd)
    if args.dryrun:
        return
    # os.system("cargo clean")
    os.system(build_cmd)


if __name__ == "__main__":
    main()
