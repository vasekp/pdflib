use std::io::Read;
use flate2::read::ZlibDecoder;

pub fn decode<R: Read>(input: R) -> ZlibDecoder<R> {
    ZlibDecoder::new(input)
}
