#[macro_use]
extern crate clap;
extern crate stateright;

mod two_phase_commit;

use clap::{Arg, App, AppSettings, SubCommand};
use stateright::*;
use std::collections::BTreeSet;
use std::iter::FromIterator;

fn main() {
    let args = App::new("bench")
        .about("benchmarks the stateright library")
        .version(crate_version!())
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .subcommand(SubCommand::with_name("2pc")
            .about("two phase commit")
            .arg(Arg::with_name("rm_count").help("number of resource managers").default_value("7")))
        .get_matches();
    match args.subcommand() {
        ("2pc", Some(args)) => {
            let rm_count = value_t!(args, "rm_count", u32).expect("rm_count");
            println!("Benchmarking two phase commit with {} resource managers.", rm_count);

            let mut sys = two_phase_commit::TwoPhaseSys {
                rms: BTreeSet::from_iter(0..rm_count)
            };
            sys.checker(KeepPaths::Yes, two_phase_commit::is_consistent).check_and_report();
        }
        _ => panic!("expected subcommand")
    }
}
