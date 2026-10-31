// Photos on the general pasteboard: file URLs for Finder, PNG for other apps.
#import <AppKit/AppKit.h>
#include <stddef.h>
#include <stdint.h>

// Replaces the pasteboard with one file URL per path. Returns the change
// count that identifies this content, or -1 when nothing was written.
long photosort_clipboard_write_files(const char *const *paths, size_t count) {
    @autoreleasepool {
        NSMutableArray<NSURL *> *urls = [NSMutableArray arrayWithCapacity:count];
        for (size_t i = 0; i < count; i++) {
            NSString *path = [NSString stringWithUTF8String:paths[i]];
            if (path == nil) {
                return -1;
            }
            [urls addObject:[NSURL fileURLWithPath:path]];
        }
        NSPasteboard *pasteboard = [NSPasteboard generalPasteboard];
        [pasteboard clearContents];
        if (![pasteboard writeObjects:urls]) {
            return -1;
        }
        return (long)[pasteboard changeCount];
    }
}

// Adds PNG data beside the file URL written with `change`. Returns 1 when
// added, 0 when something else was copied since, -1 on failure.
int photosort_clipboard_add_png(long change, const uint8_t *bytes, size_t len) {
    @autoreleasepool {
        NSPasteboard *pasteboard = [NSPasteboard generalPasteboard];
        if ((long)[pasteboard changeCount] != change) {
            return 0;
        }
        [pasteboard addTypes:@[ NSPasteboardTypePNG ] owner:nil];
        NSData *data = [NSData dataWithBytes:bytes length:len];
        return [pasteboard setData:data forType:NSPasteboardTypePNG] ? 1 : -1;
    }
}
