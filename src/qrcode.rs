use crate::errors::Error;
use consistenttime::ct_u8_slice_eq;
use image::png::PNGEncoder;
use image::{Luma, Pixel};
use qrcode::QrCode;

pub fn as_png(s: &str) -> Result<Vec<u8>, Error> {
    let qrcode = QrCode::new(s).map_err(|e| Error::General(s!(e)))?;

    let png = qrcode.render::<Luma<u8>>().build();
    let mut buf: Vec<u8> = Vec::new();
    PNGEncoder::new(&mut buf)
        .encode(&png, png.width(), png.height(), Luma::<u8>::color_type())
        .map_err(|e| Error::General(format!("Cannot write PNG file: {}", e)))?;
    Ok(buf)
}
