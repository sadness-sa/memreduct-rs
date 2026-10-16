//! Numeric notification icon, following Mem Reduct's runtime text -> bitmap -> HICON design.
use crate::{
    config::{parse_color, TraySettings},
    platform::{os_error, wide},
    AppResult,
};
use std::{
    mem::{size_of, zeroed},
    ptr::null_mut,
};
use windows_sys::Win32::{Foundation::*, Graphics::Gdi::*, UI::WindowsAndMessaging::*};

pub struct NumberIcon(pub HICON);
impl Drop for NumberIcon {
    fn drop(&mut self) {
        unsafe {
            DestroyIcon(self.0);
        }
    }
}

pub fn number_icon(load: Option<u32>, size: i32, settings: &TraySettings) -> AppResult<NumberIcon> {
    settings.validate()?;
    let color = settings.background(load)?;
    let text_color = parse_color(&settings.text_color)?;
    let size = size.clamp(16, 64);
    let label = load
        .map(|v| v.min(100).to_string())
        .unwrap_or_else(|| "--".into());
    unsafe {
        let dc = CreateCompatibleDC(null_mut());
        if dc.is_null() {
            return Err(os_error("CreateCompatibleDC"));
        }
        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = size;
        info.bmiHeader.biHeight = -size;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let mut bits = null_mut();
        let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        let mask_bytes = vec![0u8; (((size + 15) / 16) * 2 * size) as usize];
        let mask = CreateBitmap(size, size, 1, 1, mask_bytes.as_ptr().cast());
        if bitmap.is_null() || mask.is_null() || bits.is_null() {
            if !bitmap.is_null() {
                DeleteObject(bitmap);
            }
            if !mask.is_null() {
                DeleteObject(mask);
            }
            DeleteDC(dc);
            return Err(os_error("创建数字托盘位图"));
        }
        let old_bitmap = SelectObject(dc, bitmap);
        let brush = CreateSolidBrush(color);
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: size,
            bottom: size,
        };
        FillRect(dc, &rect, brush);
        DeleteObject(brush);
        SetTextColor(dc, text_color);
        SetBkMode(dc, TRANSPARENT as i32);
        let mut height = size - 1;
        let mut font;
        let text = wide(&label);
        loop {
            font = CreateFontW(
                -height,
                0,
                0,
                0,
                FW_BOLD as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                ANTIALIASED_QUALITY as u32,
                DEFAULT_PITCH as u32,
                wide("Segoe UI").as_ptr(),
            );
            if font.is_null() {
                break;
            }
            let old = SelectObject(dc, font);
            let mut extent: SIZE = zeroed();
            let measured =
                GetTextExtentPoint32W(dc, text.as_ptr(), label.len() as i32, &mut extent) != 0;
            SelectObject(dc, old);
            if (measured && extent.cx < size) || height <= size / 2 {
                break;
            }
            DeleteObject(font);
            height -= 1;
        }
        let mut icon = null_mut();
        if !font.is_null() {
            let old_font = SelectObject(dc, font);
            DrawTextW(
                dc,
                text.as_ptr(),
                -1,
                &mut rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
            );
            GdiFlush();
            for pixel in std::slice::from_raw_parts_mut(bits.cast::<u32>(), (size * size) as usize)
            {
                *pixel |= 0xff000000;
            }
            SelectObject(dc, old_font);
            DeleteObject(font);
            // CreateIconIndirect may select these bitmaps into its own DC.
            SelectObject(dc, old_bitmap);
            icon = CreateIconIndirect(&ICONINFO {
                fIcon: 1,
                xHotspot: 0,
                yHotspot: 0,
                hbmMask: mask,
                hbmColor: bitmap,
            });
        }
        SelectObject(dc, old_bitmap);
        DeleteObject(bitmap);
        DeleteObject(mask);
        DeleteDC(dc);
        if icon.is_null() {
            Err(os_error("创建数字托盘图标"))
        } else {
            Ok(NumberIcon(icon))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numeric_icons_cover_dpi_sizes_and_all_digit_counts() {
        for size in [16, 24, 32] {
            for load in [None, Some(0), Some(9), Some(85), Some(100)] {
                let icon = number_icon(load, size, &TraySettings::default()).unwrap();
                unsafe {
                    let mut info: ICONINFO = zeroed();
                    assert_ne!(GetIconInfo(icon.0, &mut info), 0);
                    let mut bitmap: BITMAP = zeroed();
                    assert_ne!(
                        GetObjectW(
                            info.hbmColor,
                            size_of::<BITMAP>() as i32,
                            (&mut bitmap as *mut BITMAP).cast()
                        ),
                        0
                    );
                    assert_eq!(bitmap.bmWidth, size);
                    assert_eq!(bitmap.bmHeight, size);
                    DeleteObject(info.hbmColor);
                    DeleteObject(info.hbmMask);
                }
            }
        }
    }
}
