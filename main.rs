use std::{env, process};
use strata_log::{Durability, Log};
fn run() -> Result<(), Box<dyn std::error::Error>> {
 let args:Vec<String>=env::args().collect();
 if args.len()<3 { return Err("usage: strata <append|dump|verify> <path> [record ...]".into()); }
 let mut log=Log::open(&args[2])?;
 match args[1].as_str() {
  "append" if args.len()>3 => { let records:Vec<Vec<u8>>=args[3..].iter().map(|x|x.as_bytes().to_vec()).collect(); println!("batch {}",log.append(&records,Durability::Sync)?); },
  "dump" if args.len()==3 => for batch in log.batches(){ for (i,r) in batch.records.iter().enumerate(){ println!("{}:{} {}",batch.sequence,i,hex(r)); } },
  "verify" if args.len()==3 => println!("ok: {} batches",log.batches().len()),
  _ => return Err("usage: strata <append|dump|verify> <path> [record ...]".into()),
 }
 Ok(())
}
fn hex(b:&[u8])->String { b.iter().map(|v|format!("{v:02x}")).collect() }
fn main(){if let Err(e)=run(){eprintln!("strata: {e}");process::exit(1)}}
