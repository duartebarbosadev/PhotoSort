// Metadata bridge; writes are allowed only on Rust-owned staging copies. Rust owns the cache and all workflow decisions.
#include <exiv2/exiv2.hpp>
#include <algorithm>
#include <cstdint>
#include <mutex>
#include <string>
#include <unordered_set>

struct PhotoSortMetadata {
    size_t fields;
    uint32_t width;
    uint32_t height;
    uint32_t writable;
};
namespace {
std::mutex metadata_mutex;
bool initialize_metadata() {
    static const bool initialized = Exiv2::XmpParser::initialize();
    return initialized;
}
}
using Emit = void (*)(void *, const char *, size_t, const char *, size_t, const char *, size_t);

extern "C" int photosort_metadata_read(const char *path, size_t length, void *context,
                                       Emit emit, PhotoSortMetadata *summary) noexcept {
    // XMP parsing uses process-wide state. Serialize access even in parallel tests
    // and library consumers; the desktop already has one shared image actor.
    try {
        std::lock_guard<std::mutex> lock(metadata_mutex);
        if (!initialize_metadata()) return 0;
        // Explicitly use local file IO: metadata paths must never select a URL backend.
        auto image = Exiv2::ImageFactory::open(std::make_unique<Exiv2::FileIo>(std::string(path, length)));
        if (!image) return 0;
        image->readMetadata();
        std::unordered_set<std::string> populated;
        const auto read = [&](const auto &data) {
            for (const auto &item : data) {
                const auto key = item.key();
                const auto raw = item.toString();
                const auto type = item.typeId();
                // Python represents XMP arrays/language alternatives as list/dict
                // values, including empty collections. Scalar empty/"None" is absent.
                const bool collection = type >= Exiv2::xmpAlt && type <= Exiv2::langAlt;
                if (collection || (!raw.empty() && raw != "None")) populated.insert(key);
                else populated.erase(key);
                static const char *selected[] = {
                    "Exif.Image.Model", "Exif.Photo.LensModel", "Exif.Photo.DateTimeOriginal",
                    "Exif.Image.DateTime", "Exif.Photo.ExposureTime", "Exif.Photo.FNumber",
                    "Exif.Photo.ISOSpeedRatings", "Exif.Image.Orientation", "Exif.Photo.PixelXDimension",
                    "Exif.Photo.PixelYDimension", "Exif.GPSInfo.GPSLatitude", "Exif.GPSInfo.GPSLatitudeRef",
                    "Exif.GPSInfo.GPSLongitude", "Exif.GPSInfo.GPSLongitudeRef",
                    "Exif.Image.Rating", "Xmp.xmp.Rating", "Xmp.xmp.CreateDate", "Xmp.photoshop.DateCreated",
                    // Details panel values, formatted on the Rust side from raw codes.
                    "Exif.Image.Make", "Exif.Image.Software", "Exif.Photo.FocalLength",
                    "Exif.Photo.ExposureBiasValue", "Exif.Photo.Flash", "Exif.Photo.WhiteBalance",
                    "Exif.Photo.MeteringMode", "Exif.Photo.ExposureMode",
                    "Exif.Photo.SceneCaptureType", "Exif.Photo.ColorSpace", "Exif.Image.BitsPerSample",
                    "Exif.Photo.LensSpecification", "Xmp.aux.Lens",
                    // Tells apart shots taken within the same second.
                    "Exif.Photo.SubSecTimeOriginal"
                };
                if (std::find(std::begin(selected), std::end(selected), key) != std::end(selected)) {
                    const auto display = item.print(&image->exifData());
                    emit(context, key.data(), key.size(), raw.data(), raw.size(), display.data(), display.size());
                }
            }
        };
        read(image->exifData());
        read(image->iptcData());
        read(image->xmpData());
        // The reference's cached dictionary also contains path, dimensions, MIME
        // and file size. Numeric zero still counts as a populated value there.
        *summary = {populated.size() + 4 + !image->mimeType().empty(),
                    image->pixelWidth(),
                    image->pixelHeight(),
                    static_cast<uint32_t>((image->checkMode(Exiv2::mdExif) & Exiv2::amWrite) &&
                                          (image->checkMode(Exiv2::mdXmp) & Exiv2::amWrite))};
        return 1;
    } catch (...) {
        // A malformed/unsupported file uses the existing Rust fallback. No C++
        // exception may cross the C boundary and no partial snapshot is published.
        return 0;
    }
}

// Never called on the source: Rust prepares a private copy, checks fingerprints,
// syncs the result and atomically commits it only after this verification succeeds.
extern "C" int photosort_metadata_write_copy(const char *path, size_t length,
                                              int rating, int orientation) noexcept {
    try {
        std::lock_guard<std::mutex> lock(metadata_mutex);
        if (!initialize_metadata() || rating < -1 || rating > 5 ||
            orientation < -1 || orientation == 0 || orientation > 8) return 0;
        auto image = Exiv2::ImageFactory::open(std::make_unique<Exiv2::FileIo>(std::string(path, length)));
        if (!image || !(image->checkMode(Exiv2::mdExif) & Exiv2::amWrite) ||
                      !(image->checkMode(Exiv2::mdXmp) & Exiv2::amWrite)) return 0;
        image->readMetadata();
        if (rating >= 0) {
            image->exifData()["Exif.Image.Rating"] = static_cast<uint16_t>(rating);
            image->xmpData()["Xmp.xmp.Rating"] = std::to_string(rating);
        }
        if (orientation >= 1) {
            image->exifData()["Exif.Image.Orientation"] = static_cast<uint16_t>(orientation);
            image->xmpData()["Xmp.tiff.Orientation"] = std::to_string(orientation);
        }
        image->writeMetadata();
        image->readMetadata();
        const auto verified = [&](const char *exif_key, const char *xmp_key, int wanted) {
            const auto exif = image->exifData().findKey(Exiv2::ExifKey(exif_key));
            const auto xmp = image->xmpData().findKey(Exiv2::XmpKey(xmp_key));
            return exif != image->exifData().end() && xmp != image->xmpData().end() &&
                   exif->toInt64() == wanted && xmp->toInt64() == wanted;
        };
        return (rating < 0 || verified("Exif.Image.Rating", "Xmp.xmp.Rating", rating)) &&
               (orientation < 0 || verified("Exif.Image.Orientation", "Xmp.tiff.Orientation", orientation));
    } catch (...) { return 0; }
}
