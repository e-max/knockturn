use crate::errors::Error;
use consistenttime::ct_u8_slice_eq;
use data_encoding::BASE32;
use image::png::PNGEncoder;
use image::{Luma, Pixel};
use qrcode::QrCode;

pub struct Totp {
    merchant: String,
    token: String,
}

impl Totp {
    pub fn new(merchant: String, token: String) -> Self {
        Totp { merchant, token }
    }

    pub fn get_png(&self) -> Result<Vec<u8>, Error> {
        let code_str = format!(
            "otpauth://totp/Knockturn:{}?secret={}&issuer=Knockturn",
            self.merchant, self.token
        );
        let qrcode = QrCode::new(&code_str).map_err(|e| Error::General(s!(e)))?;

        let png = qrcode.render::<Luma<u8>>().build();
        let mut buf: Vec<u8> = Vec::new();
        PNGEncoder::new(&mut buf)
            .encode(&png, png.width(), png.height(), Luma::<u8>::color_type())
            .map_err(|e| Error::General(format!("Cannot write PNG file: {}", e)))?;
        Ok(buf)
    }

    pub fn generate(&self) -> Result<String, Error> {
        let totp = boringauth::oath::TOTPBuilder::new()
            .base32_key(&self.token)
            .finalize()
            .map_err(|e| Error::General(format!("Got error code from boringauth {:?}", e)))?;
        Ok(totp.generate())
    }

    pub fn check(&self, code: &str) -> Result<bool, Error> {
        let corrent_code = self.generate()?;
        Ok(ct_u8_slice_eq(corrent_code.as_bytes(), code.as_bytes()))
    }
}
