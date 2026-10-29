//! A small, single-writer append-only log. See README for the precise contract.
#![forbid(unsafe_code)]
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

const MAGIC: &[u8; 8] = b"STRATA01";
const HEADER: usize = 24;
const MAX_FRAME: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch { pub sequence: u64, pub records: Vec<Vec<u8>> }

/// Controls whether successful append waits for the OS to sync file contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Durability { Buffered, Sync }

#[derive(Debug)]
pub enum Error { Io(io::Error), Corrupt { offset: u64, reason: &'static str }, TooLarge, SequenceExhausted, Poisoned }
impl std::fmt::Display for Error {
 fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
  match self { Self::Io(e) => write!(f,"I/O error: {e}"), Self::Corrupt{offset,reason} => write!(f,"corrupt log at byte {offset}: {reason}"), Self::TooLarge => write!(f,"batch exceeds format limits"), Self::SequenceExhausted => write!(f,"sequence exhausted"), Self::Poisoned => write!(f,"writer is poisoned after an I/O failure; reopen the log") }
 }
}
impl std::error::Error for Error { fn source(&self) -> Option<&(dyn std::error::Error + 'static)> { if let Self::Io(e)=self {Some(e)} else {None} } }
impl From<io::Error> for Error { fn from(e: io::Error) -> Self { Self::Io(e) } }
pub type Result<T> = std::result::Result<T, Error>;

/// Recovery truncates only an incomplete final frame. Complete corrupt frames fail closed.
/// The caller must ensure exclusive access for the entire lifetime of this handle.
pub struct Log { file: File, next: u64, batches: Vec<Batch>, poisoned: bool }
impl Log {
 pub fn open(path: impl AsRef<Path>) -> Result<Self> {
  let mut file = OpenOptions::new().read(true).write(true).create(true).open(path)?;
  let len = file.metadata()?.len();
  if len == 0 { file.write_all(MAGIC)?; file.sync_all()?; }
  else {
   let mut magic = [0u8;8];
   if len < 8 { return Err(Error::Corrupt{offset:0,reason:"incomplete file header"}); }
   file.read_exact(&mut magic)?;
   if &magic != MAGIC { return Err(Error::Corrupt{offset:0,reason:"bad file header"}); }
  }
  let mut batches=Vec::new(); let mut offset=8u64; let mut next=0u64;
  let end=file.metadata()?.len();
  while offset < end {
   let remain=end-offset;
   if remain < HEADER as u64 { file.set_len(offset)?; file.sync_all()?; break; }
   file.seek(SeekFrom::Start(offset))?;
   let mut h=[0u8;HEADER]; file.read_exact(&mut h)?;
   if &h[0..4] != b"BAT1" { return Err(Error::Corrupt{offset,reason:"bad frame magic"}); }
   let size=u32::from_le_bytes(h[4..8].try_into().unwrap()) as usize;
   let seq=u64::from_le_bytes(h[8..16].try_into().unwrap());
   let count=u32::from_le_bytes(h[16..20].try_into().unwrap()) as usize;
   let checksum=u32::from_le_bytes(h[20..24].try_into().unwrap());
   if size > MAX_FRAME { return Err(Error::Corrupt{offset,reason:"frame exceeds limit"}); }
   if seq != next { return Err(Error::Corrupt{offset,reason:"non-contiguous sequence"}); }
   if remain - (HEADER as u64) < size as u64 { file.set_len(offset)?; file.sync_all()?; break; }
   let mut body=vec![0;size]; file.read_exact(&mut body)?;
   let mut check=h[..20].to_vec(); check.extend_from_slice(&body);
   if crc32(&check) != checksum { return Err(Error::Corrupt{offset,reason:"checksum mismatch"}); }
   if count > size / 4 { return Err(Error::Corrupt{offset,reason:"invalid record count"}); }
   let mut records=Vec::with_capacity(count); let mut pos=0usize;
   for _ in 0..count {
    if size-pos < 4 { return Err(Error::Corrupt{offset,reason:"missing record length"}); }
    let n=u32::from_le_bytes(body[pos..pos+4].try_into().unwrap()) as usize; pos+=4;
    if n > size-pos { return Err(Error::Corrupt{offset,reason:"record exceeds frame"}); }
    records.push(body[pos..pos+n].to_vec()); pos+=n;
   }
   if pos != size { return Err(Error::Corrupt{offset,reason:"trailing frame bytes"}); }
   batches.push(Batch{sequence:seq,records});
   next=next.checked_add(1).ok_or(Error::SequenceExhausted)?;
   offset += HEADER as u64 + size as u64;
  }
  file.seek(SeekFrom::End(0))?;
  Ok(Self{file,next,batches,poisoned:false})
 }
 pub fn batches(&self) -> &[Batch] { &self.batches }
 pub fn append(&mut self, records: &[Vec<u8>], durability: Durability) -> Result<u64> {
  if self.poisoned { return Err(Error::Poisoned); }
  let next=self.next.checked_add(1).ok_or(Error::SequenceExhausted)?;
  let count=u32::try_from(records.len()).map_err(|_|Error::TooLarge)?;
  let mut size=0usize;
  for r in records { let _=u32::try_from(r.len()).map_err(|_|Error::TooLarge)?; size=size.checked_add(4).and_then(|v|v.checked_add(r.len())).ok_or(Error::TooLarge)?; }
  if size > MAX_FRAME { return Err(Error::TooLarge); }
  let mut frame=Vec::with_capacity(HEADER+size);
  frame.extend_from_slice(b"BAT1"); frame.extend_from_slice(&(size as u32).to_le_bytes());
  frame.extend_from_slice(&self.next.to_le_bytes()); frame.extend_from_slice(&count.to_le_bytes());
  let mut body=Vec::with_capacity(size);
  for r in records { body.extend_from_slice(&(r.len() as u32).to_le_bytes()); body.extend_from_slice(r); }
  let mut check=frame.clone(); check.extend_from_slice(&body); frame.extend_from_slice(&crc32(&check).to_le_bytes()); frame.extend_from_slice(&body);
  if let Err(e)=self.file.write_all(&frame).and_then(|_| if durability==Durability::Sync { self.file.sync_data() } else { Ok(()) }) { self.poisoned=true; return Err(Error::Io(e)); }
  let sequence=self.next; self.next=next; self.batches.push(Batch{sequence,records:records.to_vec()}); Ok(sequence)
 }
 pub fn sync(&mut self) -> Result<()> { if self.poisoned {return Err(Error::Poisoned)}; if let Err(e)=self.file.sync_data(){self.poisoned=true;return Err(Error::Io(e))} Ok(()) }
}
fn crc32(bytes: &[u8]) -> u32 { let mut c=!0u32; for &b in bytes { c ^= b as u32; for _ in 0..8 { c=(c>>1) ^ (0xedb88320 & (0u32.wrapping_sub(c&1))); } } !c }
#[cfg(test)] mod tests { use super::*; #[test] fn crc_standard_vector(){assert_eq!(crc32(b"123456789"),0xcbf43926);} }
