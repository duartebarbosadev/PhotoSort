//! One shared native metadata boundary; no file/model/cache ownership lives in the bridge.
use std::{collections::HashMap, ffi::c_void, path::Path};
#[derive(Default)]
#[repr(C)]
struct Summary {
    fields: usize,
    width: u32,
    height: u32,
    writable: u32,
}
pub(super) struct Snapshot {
    summary: Summary,
    values: HashMap<String, (String, String)>,
}
unsafe extern "C" {
    fn photosort_metadata_write_copy(
        path: *const u8,
        length: usize,
        rating: std::ffi::c_int,
        orientation: std::ffi::c_int,
    ) -> std::ffi::c_int;
    fn photosort_metadata_read(
        path: *const u8,
        length: usize,
        context: *mut c_void,
        emit: unsafe extern "C" fn(
            *mut c_void,
            *const u8,
            usize,
            *const u8,
            usize,
            *const u8,
            usize,
        ),
        summary: *mut Summary,
    ) -> std::ffi::c_int;
}
unsafe extern "C" fn emit(
    context: *mut c_void,
    key: *const u8,
    key_len: usize,
    raw: *const u8,
    raw_len: usize,
    display: *const u8,
    display_len: usize,
) {
    // SAFETY: the synchronous bridge supplies live string buffers and the exact
    // exclusive map pointer passed by read(). Nothing outlives this callback.
    unsafe {
        let text = |p, n| String::from_utf8_lossy(std::slice::from_raw_parts(p, n)).into_owned();
        let values = &mut *context.cast::<HashMap<String, (String, String)>>();
        values.insert(
            text(key, key_len),
            (text(raw, raw_len), text(display, display_len)),
        );
    }
}
pub(super) fn read(path: &Path) -> Option<Snapshot> {
    let mut snapshot = Snapshot {
        summary: Summary::default(),
        values: HashMap::new(),
    };
    let path = path.as_os_str().as_encoded_bytes();
    if path.contains(&0) {
        return None;
    }
    // SAFETY: path/context/output remain valid throughout the synchronous call;
    // the bridge catches every C++ exception and never retains borrowed pointers.
    let success = unsafe {
        photosort_metadata_read(
            path.as_ptr(),
            path.len(),
            (&mut snapshot.values as *mut HashMap<String, (String, String)>).cast(),
            emit,
            &mut snapshot.summary,
        )
    };
    (success == 1).then_some(snapshot)
}
impl Snapshot {
    fn raw(&self, key: &str) -> &str {
        self.values.get(key).map_or("", |v| v.0.as_str())
    }
    fn display(&self, key: &str) -> String {
        self.values
            .get(key)
            .map_or_else(String::new, |v| v.1.clone())
    }
    pub(super) fn apply(
        self,
        result: &mut super::Metadata,
        dates: &mut [Option<String>; 4],
        times: &mut [Option<i64>; 4],
    ) {
        result.metadata_fields = self.summary.fields;
        result.embedded_metadata_writable = self.summary.writable != 0;
        result.width = self
            .raw("Exif.Photo.PixelXDimension")
            .parse()
            .unwrap_or(self.summary.width);
        result.height = self
            .raw("Exif.Photo.PixelYDimension")
            .parse()
            .unwrap_or(self.summary.height);
        result.camera = self.raw("Exif.Image.Model").into();
        result.lens = self.raw("Exif.Photo.LensModel").into();
        result.captured = self.raw("Exif.Photo.DateTimeOriginal").into();
        result.exposure = self.display("Exif.Photo.ExposureTime");
        result.aperture = self.display("Exif.Photo.FNumber");
        result.iso = self.raw("Exif.Photo.ISOSpeedRatings").into();
        result.orientation = self
            .raw("Exif.Image.Orientation")
            .parse::<u8>()
            .unwrap_or(1)
            .clamp(1, 8);
        result.rating = self
            .raw("Xmp.xmp.Rating")
            .parse::<u8>()
            .or_else(|_| self.raw("Exif.Image.Rating").parse())
            .unwrap_or(0)
            .min(5);
        for (i, key) in [
            "Exif.Photo.DateTimeOriginal",
            "Xmp.xmp.CreateDate",
            "Exif.Image.DateTime",
            "Xmp.photoshop.DateCreated",
        ]
        .into_iter()
        .enumerate()
        {
            dates[i] = crate::core::dates::parse(self.raw(key));
            times[i] = crate::core::dates::timestamp(self.raw(key));
        }
        let coordinate = |key, reference, negative| -> Option<f64> {
            let values: Vec<_> = self
                .raw(key)
                .split_whitespace()
                .map(|v| {
                    if let Some((n, d)) = v.split_once('/') {
                        Some(n.parse::<f64>().ok()? / d.parse::<f64>().ok()?)
                    } else {
                        v.parse::<f64>().ok()
                    }
                })
                .collect::<Option<_>>()?;
            if values.len() != 3 || values.iter().any(|v| !v.is_finite()) {
                return None;
            }
            Some(
                (values[0] + values[1] / 60. + values[2] / 3600.)
                    * if self.raw(reference) == negative {
                        -1.
                    } else {
                        1.
                    },
            )
        };
        result.gps = coordinate(
            "Exif.GPSInfo.GPSLatitude",
            "Exif.GPSInfo.GPSLatitudeRef",
            "S",
        )
        .zip(coordinate(
            "Exif.GPSInfo.GPSLongitude",
            "Exif.GPSInfo.GPSLongitudeRef",
            "W",
        ));
    }
}

pub(super) fn write_copy(
    path: &Path,
    rating: Option<u8>,
    orientation: Option<u8>,
) -> anyhow::Result<()> {
    let bytes = path.as_os_str().as_encoded_bytes();
    anyhow::ensure!(!bytes.contains(&0), "Invalid metadata staging path");
    // SAFETY: the byte slice lives through this synchronous call; native code
    // retains nothing, serializes library access and catches C++ exceptions.
    let success = unsafe {
        photosort_metadata_write_copy(
            bytes.as_ptr(),
            bytes.len(),
            rating.map_or(-1, i32::from),
            orientation.map_or(-1, i32::from),
        )
    };
    anyhow::ensure!(
        success == 1,
        "Embedded metadata update or verification failed; source was not replaced"
    );
    Ok(())
}
