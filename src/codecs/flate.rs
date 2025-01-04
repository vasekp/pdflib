use std::io::{BufRead, BufReader};
use flate2::bufread::ZlibDecoder;

pub fn decode<R: BufRead>(input: R) -> BufReader<ZlibDecoder<R>> {
    std::io::BufReader::new(ZlibDecoder::new(input))
}
