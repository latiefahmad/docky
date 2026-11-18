use std::mem::size_of;

use thiserror::Error;
use windows::{
    Win32::{
        Foundation::SIZE,
        Graphics::Gdi::{
            BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC,
            GetDIBits, GetObjectW, HBITMAP, ReleaseDC,
        },
        UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_ICONONLY},
    },
    core::HSTRING,
};

#[derive(Debug, Error)]
pub enum IconError {
    #[error("the shell could not generate the icon of `{name}`")]
    Shell {
        name: String,
        #[source]
        source: windows::core::Error,
    },
    #[error("could not read the icon pixels of `{0}`")]
    Pixels(String),
}

/// Square BGRA icon with premultiplied alpha and top-to-bottom rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconImage {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

struct OwnedBitmap(HBITMAP);

impl Drop for OwnedBitmap {
    fn drop(&mut self) {
        // SAFETY: This HBITMAP is owned by us (returned by GetImage) and is not used afterward.
        unsafe {
            let _ = DeleteObject(self.0.into());
        }
    }
}

pub fn load_icon(parsing_name: &str, size: u32) -> Result<IconImage, IconError> {
    let shell_error = |source| IconError::Shell {
        name: parsing_name.to_owned(),
        source,
    };
    let side = size as i32;

    // SAFETY: The strings and structures passed to each call remain alive for its duration.
    let bitmap = unsafe {
        let factory: IShellItemImageFactory =
            SHCreateItemFromParsingName(&HSTRING::from(parsing_name), None).map_err(shell_error)?;
        OwnedBitmap(
            factory
                .GetImage(SIZE { cx: side, cy: side }, SIIGBF_ICONONLY)
                .map_err(shell_error)?,
        )
    };

    read_pixels(&bitmap).ok_or_else(|| IconError::Pixels(parsing_name.to_owned()))
}

fn read_pixels(bitmap: &OwnedBitmap) -> Option<IconImage> {
    let mut desc = BITMAP::default();
    // SAFETY: `desc` has the size specified in its size field.
    let written = unsafe {
        GetObjectW(
            bitmap.0.into(),
            size_of::<BITMAP>() as i32,
            Some((&raw mut desc).cast()),
        )
    };
    if written == 0 || desc.bmWidth <= 0 || desc.bmHeight <= 0 {
        return None;
    }
    let (width, height) = (desc.bmWidth as u32, desc.bmHeight as u32);

    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: desc.bmWidth,
            // A negative height specifies top-down rows.
            biHeight: -desc.bmHeight,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bgra = vec![0u8; (width * height * 4) as usize];
    // SAFETY: The buffer has `width * height * 4` bytes, the amount GetDIBits writes at 32 bpp.
    let lines = unsafe {
        let dc = GetDC(None);
        let lines = GetDIBits(
            dc,
            bitmap.0,
            0,
            height,
            Some(bgra.as_mut_ptr().cast()),
            &mut info,
            DIB_RGB_COLORS,
        );
        ReleaseDC(None, dc);
        lines
    };
    if lines == 0 {
        return None;
    }

    // Legacy icons without an alpha channel have an alpha value of 0 for every pixel.
    if bgra.as_chunks::<4>().0.iter().all(|px| px[3] == 0) {
        bgra.as_chunks_mut::<4>()
            .0
            .iter_mut()
            .for_each(|px| px[3] = 255);
    } else {
        premultiply(&mut bgra);
    }
    Some(IconImage {
        width,
        height,
        bgra,
    })
}

/// Premultiplies the alpha of a BGRA buffer in place, as the compositor expects.
///
/// `GetDIBits` returns straight alpha, but the icon is drawn with a
/// premultiplied-alpha bitmap (see `gfx::draw_image`). Without this, partly
/// transparent pixels (anti-aliased edges) come out too bright, which shows as
/// a halo around icons such as Command Prompt.
fn premultiply(bgra: &mut [u8]) {
    for px in bgra.as_chunks_mut::<4>().0 {
        let alpha = u32::from(px[3]);
        for channel in &mut px[..3] {
            *channel = ((u32::from(*channel) * alpha + 127) / 255) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::premultiply;

    #[test]
    fn premultiply_should_scale_color_channels_by_alpha() {
        // A fully opaque pixel is untouched; a half-transparent one is halved.
        let mut bgra = vec![200, 100, 50, 255, 200, 100, 50, 128];
        premultiply(&mut bgra);
        assert_eq!(bgra, vec![200, 100, 50, 255, 100, 50, 25, 128]);
    }

    #[test]
    fn premultiply_should_zero_out_fully_transparent_pixels() {
        let mut bgra = vec![200, 100, 50, 0];
        premultiply(&mut bgra);
        assert_eq!(bgra, vec![0, 0, 0, 0]);
    }
}
