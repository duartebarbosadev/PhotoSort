// Own the application entry point without replacing winit's application delegate.
// Winit supports an NSApplication subclass and keeps its own event dispatch.
#import <AppKit/AppKit.h>
#include <stdbool.h>

typedef void (*PhotoSortQuitCallback)(void *);
static PhotoSortQuitCallback quit_callback = NULL;
static void *quit_context = NULL;
static BOOL early_quit = NO;
static BOOL intercept_quit = YES;

@interface PhotoSortApplication : NSApplication
@end
@implementation PhotoSortApplication
- (void)terminate:(id)sender {
    if (!intercept_quit) {
        [super terminate:sender];
        return;
    }
    // Native Quit (including Cmd+Q) only requests departure. Rust owns prompts,
    // worker shutdown and the eventual authorized window close.
    if (quit_callback) quit_callback(quit_context);
    else early_quit = YES;
}
@end

bool photosort_application_initialize(void) {
    if (![NSThread isMainThread]) return false;
    if (NSApp && ![NSApp isKindOfClass:[PhotoSortApplication class]]) return false;
    @autoreleasepool {
        [PhotoSortApplication sharedApplication];
        return [NSApp isKindOfClass:[PhotoSortApplication class]];
    }
}
bool photosort_quit_install(PhotoSortQuitCallback callback, void *context) {
    if (![NSThread isMainThread] || !callback || quit_callback ||
        ![NSApp isKindOfClass:[PhotoSortApplication class]]) return false;
    quit_context = context;
    quit_callback = callback;
    intercept_quit = YES;
    if (early_quit) {
        early_quit = NO;
        quit_callback(quit_context);
    }
    return true;
}
void photosort_quit_remove(void *context) {
    if (![NSThread isMainThread] || quit_context != context) return;
    quit_callback = NULL;
    quit_context = NULL;
    early_quit = NO;
    intercept_quit = NO;
}
