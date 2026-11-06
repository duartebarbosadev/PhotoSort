#include "apple_image.h"
#import <Foundation/Foundation.h>
#include <CoreFoundation/CoreFoundation.h>
#include <CoreGraphics/CoreGraphics.h>
#include <ImageIO/ImageIO.h>
#include <Accelerate/Accelerate.h>
#include <stdlib.h>
#include <stdint.h>

static unsigned int dimension(CFDictionaryRef properties, CFStringRef key) {
    int value = 0;
    CFTypeRef number = properties ? CFDictionaryGetValue(properties, key) : NULL;
    if (number && CFGetTypeID(number) == CFNumberGetTypeID())
        CFNumberGetValue((CFNumberRef)number, kCFNumberIntType, &value);
    return value > 0 ? (unsigned int)value : 0;
}
/* No real photo is larger; a header claiming more is corrupt or hostile. */
#define MAX_PIXELS (1ULL << 32)
/* Previews run unattended: ImageIO decodes the whole HEVC image to make one,
   so past a gigapixel a hostile header would hold memory and time. */
#define MAX_PREVIEW_PIXELS (1ULL << 30)
/* Peak bytes per pixel of a full-size decode: ImageIO's image, the RGBA
   bitmap it is drawn into and the caller's copy of that. */
#define FULL_BYTES_PER_PIXEL 12
static unsigned char *decode(const unsigned char *path, long path_len, unsigned int edge,
    unsigned long long max_bytes, unsigned int *width, unsigned int *height,
    unsigned int *source_width, unsigned int *source_height) {
    *width = *height = *source_width = *source_height = 0;
    CFURLRef url = CFURLCreateFromFileSystemRepresentation(NULL, path, path_len, false);
    if (!url) return NULL;
    const void *source_keys[] = {kCGImageSourceShouldCache};
    const void *source_values[] = {kCFBooleanFalse};
    CFDictionaryRef options = CFDictionaryCreate(NULL, source_keys, source_values, 1,
        &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    CGImageSourceRef source = CGImageSourceCreateWithURL(url, options);
    CFRelease(url);
    if (!source) { CFRelease(options); return NULL; }
    CFDictionaryRef properties = CGImageSourceCopyPropertiesAtIndex(source, 0, NULL);
    unsigned int sw = dimension(properties, kCGImagePropertyPixelWidth);
    unsigned int sh = dimension(properties, kCGImagePropertyPixelHeight);
    if (properties) CFRelease(properties);
    /* An unknown size cannot be checked, so it is not decoded. */
    if (!sw || !sh || (uint64_t)sw * sh > MAX_PIXELS) { CFRelease(source); CFRelease(options); return NULL; }
    const int reduced = edge && (sw > edge || sh > edge);
    if (reduced && (uint64_t)sw * sh > MAX_PREVIEW_PIXELS) { CFRelease(source); CFRelease(options); return NULL; }
    /* A reduced thumbnail is bounded by its edge; only a full-size decode
       holds the whole image, so only it is bounded by memory. The caller
       reads the source size to say how much it would need. */
    if (!reduced && (uint64_t)sw * sh * FULL_BYTES_PER_PIXEL > max_bytes) {
        *source_width = sw; *source_height = sh;
        CFRelease(source); CFRelease(options); return NULL;
    }
    CGImageRef image = NULL;
    if (reduced) {
        int maximum = (int)edge;
        CFNumberRef size = CFNumberCreate(NULL, kCFNumberIntType, &maximum);
        const void *keys[] = {kCGImageSourceThumbnailMaxPixelSize,
            kCGImageSourceCreateThumbnailFromImageAlways, kCGImageSourceCreateThumbnailWithTransform,
            kCGImageSourceShouldCache};
        const void *values[] = {size, kCFBooleanTrue, kCFBooleanFalse, kCFBooleanFalse};
        CFDictionaryRef thumbnail_options = CFDictionaryCreate(NULL, keys, values, 4,
            &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
        image = CGImageSourceCreateThumbnailAtIndex(source, 0, thumbnail_options);
        CFRelease(thumbnail_options); CFRelease(size);
    } else image = CGImageSourceCreateImageAtIndex(source, 0, options);
    CFRelease(source); CFRelease(options);
    if (!image) return NULL;
    size_t w = CGImageGetWidth(image), h = CGImageGetHeight(image);
    /* The image may not match its properties: check what was made. */
    if (!w || !h || (uint64_t)w * h > MAX_PIXELS || (reduced && (w > edge || h > edge)) ||
        (!reduced && (uint64_t)w * h * FULL_BYTES_PER_PIXEL > max_bytes)) {
        if (!reduced) { *source_width = (unsigned int)w; *source_height = (unsigned int)h; }
        CGImageRelease(image); return NULL;
    }
    if (!reduced) { sw = (unsigned int)w; sh = (unsigned int)h; }
    unsigned char *pixels = calloc(w * h, 4);
    if (!pixels) { CGImageRelease(image); return NULL; }
    CGColorSpaceRef space = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    if (!space) { free(pixels); CGImageRelease(image); return NULL; }
    CGContextRef context = CGBitmapContextCreate(pixels, w, h, 8, w * 4, space,
        kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big);
    CGColorSpaceRelease(space);
    if (!context) { free(pixels); CGImageRelease(image); return NULL; }
    CGImageAlphaInfo alpha = CGImageGetAlphaInfo(image);
    CGContextDrawImage(context, CGRectMake(0, 0, w, h), image);
    CGContextRelease(context); CGImageRelease(image);
    if (alpha != kCGImageAlphaNone && alpha != kCGImageAlphaNoneSkipFirst && alpha != kCGImageAlphaNoneSkipLast) {
        vImage_Buffer buffer = {pixels, h, w, w * 4};
        if (vImageUnpremultiplyData_RGBA8888(&buffer, &buffer, kvImageNoFlags) != kvImageNoError) {
            free(pixels); return NULL;
        }
    }
    *width = (unsigned int)w; *height = (unsigned int)h;
    *source_width = sw; *source_height = sh;
    return pixels;
}
/* Returned pixels are malloc-owned; no autoreleased object crosses the Rust boundary. */
unsigned char *photosort_apple_decode(const unsigned char *path, long path_len, unsigned int edge,
    unsigned long long max_bytes, unsigned int *width, unsigned int *height,
    unsigned int *source_width, unsigned int *source_height) {
    @autoreleasepool {
        return decode(path, path_len, edge, max_bytes, width, height, source_width, source_height);
    }
}
void photosort_apple_free(unsigned char *pixels) { free(pixels); }
unsigned int photosort_apple_orientation(const unsigned char *path, long path_len) {
    @autoreleasepool {
        unsigned int orientation = 1;
        CFURLRef url = CFURLCreateFromFileSystemRepresentation(NULL, path, path_len, false);
        if (!url) return orientation;
        CGImageSourceRef source = CGImageSourceCreateWithURL(url, NULL); CFRelease(url);
        if (!source) return orientation;
        CFDictionaryRef properties = CGImageSourceCopyPropertiesAtIndex(source, 0, NULL); CFRelease(source);
        unsigned int value = dimension(properties, kCGImagePropertyOrientation);
        if (value >= 1 && value <= 8) orientation = value;
        if (properties) CFRelease(properties);
        return orientation;
    }
}
