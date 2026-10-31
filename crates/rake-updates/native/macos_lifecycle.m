#import <Cocoa/Cocoa.h>

typedef void (*RakeQuitRequest)(void *context);
@interface RakeQuitDelegate : NSObject <NSApplicationDelegate>
@property(nonatomic,strong) id original;
@property(nonatomic,assign) RakeQuitRequest callback;
@property(nonatomic,assign) void *context;
@property(nonatomic,assign) BOOL pending;
@property(nonatomic,assign) BOOL approved;
@end
@implementation RakeQuitDelegate
- (BOOL)respondsToSelector:(SEL)selector {
    return [super respondsToSelector:selector] || [self.original respondsToSelector:selector];
}
- (id)forwardingTargetForSelector:(SEL)selector {
    if ([self.original respondsToSelector:selector]) return self.original;
    return [super forwardingTargetForSelector:selector];
}
- (NSApplicationTerminateReply)applicationShouldTerminate:(NSApplication *)application {
    if (self.approved) return NSTerminateNow;
    if (!self.callback) return NSTerminateCancel;
    if (!self.pending) {
        self.pending=YES;
        self.callback(self.context);
    }
    return NSTerminateLater;
}
@end

void *rake_native_quit_create(RakeQuitRequest callback, void *context) {
    if (![NSThread isMainThread] || !NSApp || !callback) return NULL;
    id original=NSApp.delegate;
    // The pinned GPUI delegate has no termination veto. Keep foreign owners intact.
    if ([original respondsToSelector:@selector(applicationShouldTerminate:)]) return NULL;
    RakeQuitDelegate *proxy=[RakeQuitDelegate new];
    proxy.original=original;
    proxy.callback=callback;
    proxy.context=context;
    NSApp.delegate=proxy;
    return (__bridge_retained void *)proxy;
}
bool rake_native_quit_active(void *handle) {return [NSThread isMainThread] && NSApp.delegate==(__bridge id)handle;}
void rake_native_quit_prepare(void *handle, bool approved) {
    RakeQuitDelegate *proxy=(__bridge RakeQuitDelegate *)handle;
    proxy.approved=approved;
}
void rake_native_quit_reply(void *handle, bool approved) {
    RakeQuitDelegate *proxy=(__bridge RakeQuitDelegate *)handle;
    proxy.approved=approved;
    if (proxy.pending) {
        proxy.pending=NO;
        [NSApp replyToApplicationShouldTerminate:approved];
    }
}
void rake_native_quit_destroy(void *handle) {
    RakeQuitDelegate *proxy=(__bridge_transfer RakeQuitDelegate *)handle;
    proxy.callback=NULL;
    proxy.context=NULL;
    if (NSApp.delegate==proxy) NSApp.delegate=proxy.original;
    if (proxy.pending) [NSApp replyToApplicationShouldTerminate:NO];
}

#ifdef RAKE_NATIVE_LIFECYCLE_TEST
@interface RakeTestQuitDelegate : NSObject <NSApplicationDelegate>
- (NSInteger)rakeForwardingProbe;
@end
@implementation RakeTestQuitDelegate
- (NSInteger)rakeForwardingProbe {return 42;}
@end
size_t rake_native_quit_test_window_count(void) {return NSApp.windows.count;}
void *rake_native_quit_test_app_create(void) {
    [NSApplication sharedApplication];
    RakeTestQuitDelegate *delegate=[RakeTestQuitDelegate new];
    NSApp.delegate=delegate;
    return (__bridge_retained void *)delegate;
}
void rake_native_quit_test_app_destroy(void *handle) {
    id delegate=(__bridge_transfer id)handle;
    if (NSApp.delegate==delegate) NSApp.delegate=nil;
}
void rake_native_quit_test_request(void) {
    dispatch_async(dispatch_get_main_queue(), ^{
        rake_native_quit_reply((__bridge void *)NSApp.delegate, false);
    });
    [NSApp terminate:nil];
}
long rake_native_quit_test_reply(void) {return [NSApp.delegate applicationShouldTerminate:NSApp];}
long rake_native_quit_test_forward(void) {return [(id)NSApp.delegate rakeForwardingProbe];}
bool rake_native_quit_test_restored(void *handle) {return NSApp.delegate==(__bridge id)handle;}
#endif
