#import <AVFoundation/AVFoundation.h>
#import <CoreVideo/CoreVideo.h>
#import <Accelerate/Accelerate.h>
#include <math.h>
#include <stdint.h>

@interface PSVideo : NSObject
@property(strong) AVURLAsset *asset;
@property(strong) AVPlayerItem *item;
@property(strong) AVPlayer *player;
@property(strong) AVPlayerItemVideoOutput *output;
@property(assign) uint64_t seekRevision;
@property(assign) BOOL seeking;
@property(assign) BOOL seekFailed;
@end
@implementation PSVideo
@end
static void message(char *out, size_t size, NSString *text) {
    if (out && size) snprintf(out, size, "%s", (text ?: @"Video playback failed").UTF8String);
}
static AVURLAsset *local_asset(const char *path) {
    NSURL *url = [NSURL fileURLWithFileSystemRepresentation:path isDirectory:NO relativeToURL:nil];
    return [AVURLAsset URLAssetWithURL:url options:@{
        AVURLAssetPreferPreciseDurationAndTimingKey:@NO,
        AVURLAssetReferenceRestrictionsKey:@(AVAssetReferenceRestrictionForbidAll)
    }];
}
void *ps_video_open(const char *path, int (*cancelled)(void *), void *context, char *error, size_t error_size) {
    @autoreleasepool {
        PSVideo *v = [PSVideo new];
        v.asset = local_asset(path);
        dispatch_semaphore_t loaded = dispatch_semaphore_create(0);
        [v.asset loadValuesAsynchronouslyForKeys:@[@"tracks",@"duration"] completionHandler:^{dispatch_semaphore_signal(loaded);}];
        CFAbsoluteTime deadline=CFAbsoluteTimeGetCurrent()+10;
        while (dispatch_semaphore_wait(loaded,dispatch_time(DISPATCH_TIME_NOW,10*NSEC_PER_MSEC))) {
            if (cancelled(context) || CFAbsoluteTimeGetCurrent()>deadline) {
                [v.asset cancelLoading]; message(error,error_size,@"Video opening cancelled or timed out"); return NULL;
            }
            CFRunLoopRunInMode(kCFRunLoopDefaultMode,0,false);
        }
        if (cancelled(context)) { [v.asset cancelLoading]; message(error,error_size,@"Video opening cancelled");return NULL; }
        AVAssetTrack *track = [v.asset tracksWithMediaType:AVMediaTypeVideo].firstObject;
        double duration = CMTimeGetSeconds(v.asset.duration);
        if (!track || !isfinite(duration) || duration <= 0) {
            message(error,error_size,@"This file has no playable finite video track"); return NULL;
        }
        CGRect bounds = CGRectApplyAffineTransform((CGRect){CGPointZero,track.naturalSize},track.preferredTransform);
        if (!isfinite(bounds.size.width) || !isfinite(bounds.size.height) || bounds.size.width <= 0 || bounds.size.height <= 0) {
            message(error,error_size,@"Invalid video dimensions"); return NULL;
        }
        double scale = fmin(1.0,1280.0/fmax(bounds.size.width,bounds.size.height));
        CGSize size = CGSizeMake(fmax(1,floor(bounds.size.width*scale)),fmax(1,floor(bounds.size.height*scale)));
        CGAffineTransform transform = CGAffineTransformConcat(track.preferredTransform,CGAffineTransformMakeTranslation(-bounds.origin.x,-bounds.origin.y));
        transform = CGAffineTransformConcat(transform,CGAffineTransformMakeScale(scale,scale));
        AVMutableVideoCompositionLayerInstruction *layer = [AVMutableVideoCompositionLayerInstruction videoCompositionLayerInstructionWithAssetTrack:track];
        [layer setTransform:transform atTime:kCMTimeZero];
        AVMutableVideoCompositionInstruction *instruction = [AVMutableVideoCompositionInstruction videoCompositionInstruction];
        instruction.timeRange = CMTimeRangeMake(kCMTimeZero,v.asset.duration);
        instruction.layerInstructions = @[layer];
        AVMutableVideoComposition *composition = [AVMutableVideoComposition videoComposition];
        composition.renderSize = size;
        composition.frameDuration = CMTimeMake(1,30);
        composition.instructions = @[instruction];
        v.item = [AVPlayerItem playerItemWithAsset:v.asset];
        v.item.videoComposition = composition;
        v.output = [[AVPlayerItemVideoOutput alloc] initWithPixelBufferAttributes:@{(id)kCVPixelBufferPixelFormatTypeKey:@(kCVPixelFormatType_32BGRA)}];
        [v.item addOutput:v.output];
        v.player = [AVPlayer playerWithPlayerItem:v.item];
        v.player.volume = 0.5;
        v.player.actionAtItemEnd = AVPlayerActionAtItemEndPause;
        return (__bridge_retained void *)v;
    }
}
void ps_video_close(void *handle) {
    @autoreleasepool {
        PSVideo *v = (__bridge_transfer PSVideo *)handle;
        [v.player pause]; [v.item cancelPendingSeeks]; [v.asset cancelLoading];
        [v.item removeOutput:v.output]; [v.player replaceCurrentItemWithPlayerItem:nil];
    }
}
void ps_video_control(void *handle, int play, double seek, float volume) {
    @autoreleasepool {
        PSVideo *v = (__bridge PSVideo *)handle;
        v.player.volume = fminf(1,fmaxf(0,volume));
        if (isfinite(seek) && seek >= 0) {
            uint64_t revision;
            @synchronized(v) { revision = ++v.seekRevision; v.seeking = YES; v.seekFailed = NO; }
            __weak PSVideo *weakVideo = v;
            [v.player seekToTime:CMTimeMakeWithSeconds(seek,600) toleranceBefore:kCMTimeZero toleranceAfter:kCMTimeZero completionHandler:^(BOOL finished) {
                PSVideo *video = weakVideo;
                if (!video) return;
                @synchronized(video) {
                    if (video.seekRevision == revision) {
                        video.seeking = NO;
                        video.seekFailed = !finished;
                    }
                }
            }];
        }
        if (play) [v.player play]; else [v.player pause];
        [v.output requestNotificationOfMediaDataChangeWithAdvanceInterval:0.03];
    }
}
typedef struct {
    double position, duration;
    uint32_t width,height;
    int ready,playing,ended,seeking;
} PSVideoStatus;
int ps_video_poll(void *handle, PSVideoStatus *status, uint8_t *pixels, size_t capacity, char *error, size_t error_size) {
    @autoreleasepool {
        CFRunLoopRunInMode(kCFRunLoopDefaultMode,0,false);
        PSVideo *v = (__bridge PSVideo *)handle;
        if (v.item.status == AVPlayerItemStatusFailed || v.player.status == AVPlayerStatusFailed) {
            message(error,error_size,(v.item.error ?: v.player.error).localizedDescription); return -1;
        }
        @synchronized(v) {
            status->seeking = v.seeking;
            if (v.seekFailed) { message(error,error_size,@"Video seek interrupted"); return -1; }
        }
        status->position = CMTimeGetSeconds(v.item.currentTime);
        status->duration = CMTimeGetSeconds(v.item.duration);
        if (!isfinite(status->position)) status->position=0;
        if (!isfinite(status->duration)) status->duration=0;
        status->ready = v.item.status == AVPlayerItemStatusReadyToPlay;
        status->playing = v.player.timeControlStatus == AVPlayerTimeControlStatusPlaying;
        status->ended = status->duration>0 && status->position>=status->duration-0.02 && !status->playing;
        CMTime time = v.item.currentTime;
        if (![v.output hasNewPixelBufferForItemTime:time]) return 0;
        CVPixelBufferRef buffer = [v.output copyPixelBufferForItemTime:time itemTimeForDisplay:NULL];
        if (!buffer) return 0;
        size_t width=CVPixelBufferGetWidth(buffer),height=CVPixelBufferGetHeight(buffer);
        if (!width || !height || width>1280 || height>1280 || width*height*4>capacity || CVPixelBufferGetPixelFormatType(buffer)!=kCVPixelFormatType_32BGRA) {
            CVPixelBufferRelease(buffer); message(error,error_size,@"Video frame exceeds the playback surface limit"); return -1;
        }
        CVReturn lock = CVPixelBufferLockBaseAddress(buffer,kCVPixelBufferLock_ReadOnly);
        if (lock!=kCVReturnSuccess) {CVPixelBufferRelease(buffer);message(error,error_size,@"Cannot read video frame");return -1;}
        vImage_Buffer input = {CVPixelBufferGetBaseAddress(buffer),height,width,CVPixelBufferGetBytesPerRow(buffer)};
        vImage_Buffer output = {pixels,height,width,width*4};
        const uint8_t permute[4]={2,1,0,3};
        vImage_Error result=vImagePermuteChannels_ARGB8888(&input,&output,permute,kvImageNoFlags);
        CVPixelBufferUnlockBaseAddress(buffer,kCVPixelBufferLock_ReadOnly); CVPixelBufferRelease(buffer);
        if (result!=kvImageNoError) {message(error,error_size,@"Cannot convert video frame");return -1;}
        status->width=(uint32_t)width;status->height=(uint32_t)height; return 1;
    }
}

@interface PSPoster : NSObject
@property(assign) CGImageRef image;
@property(strong) NSError *error;
@end
@implementation PSPoster
- (void)dealloc {if (_image) CGImageRelease(_image);}
@end
int ps_video_thumbnail(const char *path, int (*cancelled)(void *), void *cancel_context, uint8_t *pixels, size_t capacity, uint32_t *width, uint32_t *height, char *error, size_t error_size) {
    @autoreleasepool {
        AVURLAsset *asset=local_asset(path);
        AVAssetImageGenerator *generator=[AVAssetImageGenerator assetImageGeneratorWithAsset:asset];
        generator.appliesPreferredTrackTransform=YES;
        generator.maximumSize=CGSizeMake(1280,1280);
        PSPoster *poster=[PSPoster new];
        dispatch_semaphore_t done=dispatch_semaphore_create(0);
        [generator generateCGImagesAsynchronouslyForTimes:@[[NSValue valueWithCMTime:kCMTimeZero]] completionHandler:^(CMTime requested,CGImageRef image,CMTime actual,AVAssetImageGeneratorResult result,NSError *failure){
            (void)requested; (void)actual; (void)result;
            if (image) poster.image=CGImageRetain(image);
            poster.error=failure;dispatch_semaphore_signal(done);
        }];
        CFAbsoluteTime deadline=CFAbsoluteTimeGetCurrent()+10;
        while (dispatch_semaphore_wait(done,dispatch_time(DISPATCH_TIME_NOW,10*NSEC_PER_MSEC))) {
            if (cancelled(cancel_context) || CFAbsoluteTimeGetCurrent()>deadline) {
                [generator cancelAllCGImageGeneration];[asset cancelLoading];
                message(error,error_size,@"Video thumbnail cancelled or timed out");return -1;
            }
        }
        if (cancelled(cancel_context)) {message(error,error_size,@"Video thumbnail cancelled");return -1;}
        if (!poster.image) {message(error,error_size,poster.error.localizedDescription);return -1;}
        size_t w=CGImageGetWidth(poster.image),h=CGImageGetHeight(poster.image);
        if (!w || !h || w>1280 || h>1280 || w*h*4>capacity) {message(error,error_size,@"Video thumbnail exceeds its surface limit");return -1;}
        CGColorSpaceRef space=CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
        CGContextRef context=CGBitmapContextCreate(pixels,w,h,8,w*4,space,kCGImageAlphaPremultipliedLast|kCGBitmapByteOrder32Big);
        CGColorSpaceRelease(space);
        if (!context) {message(error,error_size,@"Cannot create thumbnail surface");return -1;}
        CGContextDrawImage(context,CGRectMake(0,0,w,h),poster.image);CGContextRelease(context);
        *width=(uint32_t)w;*height=(uint32_t)h;return 0;
    }
}
