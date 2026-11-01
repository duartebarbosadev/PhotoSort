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

// The close, minimise and zoom buttons sit inside the app's own top bar: the
// window draws behind a transparent title bar, and the buttons are centred in
// a bar `height` points tall, the first `inset` points from the left edge.
// AppKit lays them out again on resize and focus changes, so the app calls
// this every frame. Returns whether a window's buttons were placed.
bool photosort_window_place_buttons(double height, double inset) {
    if (![NSThread isMainThread]) return false;
    bool placed = false;
    @autoreleasepool {
        for (NSWindow *window in [NSApp windows]) {
            if (!(window.styleMask & NSWindowStyleMaskFullSizeContentView) ||
                (window.styleMask & NSWindowStyleMaskFullScreen)) continue;
            NSButton *close = [window standardWindowButton:NSWindowCloseButton];
            NSButton *mini = [window standardWindowButton:NSWindowMiniaturizeButton];
            NSButton *zoom = [window standardWindowButton:NSWindowZoomButton];
            NSView *container = close.superview.superview;
            if (!close || !mini || !zoom || !container) continue;
            NSRect frame = container.frame;
            frame.size.height = height;
            frame.origin.y = NSHeight(window.frame) - height;
            if (!NSEqualRects(frame, container.frame)) container.frame = frame;
            CGFloat spacing = NSMinX(mini.frame) - NSMinX(close.frame);
            NSArray<NSButton *> *buttons = @[close, mini, zoom];
            for (NSUInteger i = 0; i < buttons.count; i++) {
                NSButton *button = buttons[i];
                NSPoint origin = NSMakePoint(inset + i * spacing,
                                             round((height - NSHeight(button.frame)) / 2.0));
                if (!NSEqualPoints(origin, button.frame.origin)) [button setFrameOrigin:origin];
            }
            placed = true;
        }
    }
    return placed;
}

// A double-click on the top bar does what the user chose for title bars in
// System Settings: zoom (the default), minimise, or nothing.
void photosort_window_title_double_click(void) {
    if (![NSThread isMainThread]) return;
    NSWindow *window = [NSApp keyWindow] ?: [NSApp mainWindow];
    if (!window) return;
    NSUserDefaults *defaults = [NSUserDefaults standardUserDefaults];
    NSString *action = [defaults stringForKey:@"AppleActionOnDoubleClick"];
    if ([action isEqualToString:@"None"]) return;
    if ([action isEqualToString:@"Minimize"] ||
        (!action && [defaults boolForKey:@"AppleMiniaturizeOnDoubleClick"])) {
        [window performMiniaturize:nil];
    } else {
        [window performZoom:nil];
    }
}
