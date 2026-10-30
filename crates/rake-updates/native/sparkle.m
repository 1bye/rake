#import <Cocoa/Cocoa.h>
#import <Sparkle/Sparkle.h>
#include <stdio.h>

typedef void (*RakeSparkleNotify)(void *context);
@class RakeSparkleBridge;
// Sparkle and its UI may retain replies beyond controller cancellation.
@interface RakeSparkleChoice : NSObject
@property(nonatomic,copy) void (^reply)(SPUUserUpdateChoice);
- (void)finish:(SPUUserUpdateChoice)choice;
@end
@implementation RakeSparkleChoice
- (void)finish:(SPUUserUpdateChoice)choice {
    void (^reply)(SPUUserUpdateChoice)=self.reply;
    self.reply=nil;
    if (reply) reply(choice);
}
@end
// All protocol methods not intercepted here are forwarded to Sparkle's standard driver.
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wprotocol"
@interface RakeSparkleDriver : NSObject <SPUUserDriver>
@property(nonatomic,strong) SPUStandardUserDriver *standard;
@property(nonatomic,weak) RakeSparkleBridge *owner;
@end
#pragma clang diagnostic pop
@interface RakeSparkleBridge : NSObject <SPUUpdaterDelegate>
@property(nonatomic,strong) SPUUpdater *updater;
@property(nonatomic,strong) RakeSparkleDriver *driver;
@property(nonatomic,copy) NSString *feed;
@property(nonatomic,copy) NSString *lastError;
@property(nonatomic,copy) void (^pending)(BOOL);
@property(nonatomic,strong) RakeSparkleChoice *choice;
@property(nonatomic,copy) void (^onQuit)(void);
@property(nonatomic,copy) void (^cancelDownload)(void);
@property(nonatomic,assign) RakeSparkleNotify notify;
@property(nonatomic,assign) void *context;
@property(nonatomic,assign) BOOL taken;
@property(nonatomic,assign) BOOL authorized;
@property(nonatomic,assign) uint32_t stage;
@property(nonatomic,assign) uint64_t received;
@property(nonatomic,assign) uint64_t total;
@property(nonatomic,assign) double extraction;
- (void)changed;
- (void)queue:(void (^)(BOOL))resume;
@end
@implementation RakeSparkleBridge
- (void)changed {if (self.notify) self.notify(self.context);}
- (void)queue:(void (^)(BOOL))resume {
    if (!self.notify) {resume(NO);return;}
    if (self.pending) {resume(NO);return;}
    self.pending=resume;
    self.taken=NO;
    self.stage=4;
    [self changed];
}
- (NSString *)feedURLStringForUpdater:(SPUUpdater *)updater {return self.feed;}
- (BOOL)updaterShouldPromptForPermissionToCheckForUpdates:(SPUUpdater *)updater {return NO;}
- (BOOL)updater:(SPUUpdater *)updater shouldPostponeRelaunchForUpdate:(SUAppcastItem *)item untilInvokingBlock:(void (^)(void))handler {
    if (self.authorized) return NO;
    [self queue:^(BOOL approved) {if (approved) handler();}];
    return YES;
}
- (BOOL)updater:(SPUUpdater *)updater willInstallUpdateOnQuit:(SUAppcastItem *)item immediateInstallationBlock:(void (^)(void))handler {
    self.onQuit=handler;
    return YES;
}
- (BOOL)updaterShouldRelaunchApplication:(SPUUpdater *)updater {return self.authorized;}
@end
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wprotocol"
@implementation RakeSparkleDriver
- (BOOL)respondsToSelector:(SEL)selector {return [super respondsToSelector:selector] || [self.standard respondsToSelector:selector];}
- (id)forwardingTargetForSelector:(SEL)selector {return [self.standard respondsToSelector:selector] ? self.standard : [super forwardingTargetForSelector:selector];}
- (void)showUpdateFoundWithAppcastItem:(SUAppcastItem *)item state:(SPUUserUpdateState *)state reply:(void (^)(SPUUserUpdateChoice))reply {
    RakeSparkleChoice *continuation=[RakeSparkleChoice new];
    continuation.reply=reply;
    self.owner.choice=continuation;
    __weak RakeSparkleBridge *owner=self.owner;
    [self.standard showUpdateFoundWithAppcastItem:item state:state reply:^(SPUUserUpdateChoice choice) {
        if (choice==SPUUserUpdateChoiceInstall && state.stage==SPUUserUpdateStageInstalling) {
            RakeSparkleBridge *strong=owner;
            if (!strong) {[continuation finish:SPUUserUpdateChoiceSkip];return;}
            [strong queue:^(BOOL approved){[continuation finish:approved ? SPUUserUpdateChoiceInstall : SPUUserUpdateChoiceSkip];}];
        } else {[continuation finish:choice];}
    }];
}
- (void)showReadyToInstallAndRelaunch:(void (^)(SPUUserUpdateChoice))reply {
    RakeSparkleChoice *continuation=[RakeSparkleChoice new];
    continuation.reply=reply;
    self.owner.choice=continuation;
    __weak RakeSparkleBridge *owner=self.owner;
    [self.standard showReadyToInstallAndRelaunch:^(SPUUserUpdateChoice choice){
        if (choice==SPUUserUpdateChoiceInstall) {
            RakeSparkleBridge *strong=owner;
            if (!strong) {[continuation finish:SPUUserUpdateChoiceSkip];return;}
            [strong queue:^(BOOL approved){[continuation finish:approved ? SPUUserUpdateChoiceInstall : SPUUserUpdateChoiceSkip];}];
        } else {[continuation finish:choice];}
    }];
}
- (void)showUserInitiatedUpdateCheckWithCancellation:(void (^)(void))cancellation {
    self.owner.stage=1;
    self.owner.cancelDownload=cancellation;
    [self.owner changed];
    [self.standard showUserInitiatedUpdateCheckWithCancellation:cancellation];
}
- (void)showDownloadInitiatedWithCancellation:(void (^)(void))cancellation {
    self.owner.stage=2;
    self.owner.received=0;
    self.owner.total=0;
    self.owner.cancelDownload=cancellation;
    [self.owner changed];
    [self.standard showDownloadInitiatedWithCancellation:cancellation];
}
- (void)showDownloadDidReceiveExpectedContentLength:(uint64_t)length {self.owner.total=length;[self.owner changed];[self.standard showDownloadDidReceiveExpectedContentLength:length];}
- (void)showDownloadDidReceiveDataOfLength:(uint64_t)length {self.owner.received+=length;[self.owner changed];[self.standard showDownloadDidReceiveDataOfLength:length];}
- (void)showDownloadDidStartExtractingUpdate {self.owner.cancelDownload=nil;self.owner.stage=3;[self.owner changed];[self.standard showDownloadDidStartExtractingUpdate];}
- (void)showExtractionReceivedProgress:(double)progress {self.owner.extraction=progress;[self.owner changed];[self.standard showExtractionReceivedProgress:progress];}
- (void)showInstallingUpdateWithApplicationTerminated:(BOOL)terminated retryTerminatingApplication:(void (^)(void))retry {
    if (!terminated && self.owner.authorized) {self.owner.stage=7;[self.owner changed];}
    [self.standard showInstallingUpdateWithApplicationTerminated:terminated retryTerminatingApplication:retry];
}
- (void)showUpdaterError:(NSError *)error acknowledgement:(void (^)(void))acknowledgement {
    self.owner.stage=5;self.owner.lastError=error.localizedDescription;[self.owner changed];[self.standard showUpdaterError:error acknowledgement:acknowledgement];
}
- (void)showUpdateNotFoundWithError:(NSError *)error acknowledgement:(void (^)(void))acknowledgement {self.owner.stage=6;[self.owner changed];[self.standard showUpdateNotFoundWithError:error acknowledgement:acknowledgement];}
- (void)dismissUpdateInstallation {self.owner.cancelDownload=nil;if (self.owner.stage!=5 && self.owner.stage!=6) self.owner.stage=0;[self.owner changed];[self.standard dismissUpdateInstallation];}
@end
#pragma clang diagnostic pop

void *rake_sparkle_create(const char *identifier,const char *feed,const char *key,RakeSparkleNotify notify,void *context,char *error,size_t capacity) {
    @try {
        if (![NSThread isMainThread] || !NSApp) {snprintf(error,capacity,"Sparkle requires a running AppKit main thread");return NULL;}
        NSBundle *bundle=NSBundle.mainBundle;
        NSString *expected=[NSString stringWithUTF8String:key];
        if (![bundle.bundleIdentifier isEqual:[NSString stringWithUTF8String:identifier]]) {snprintf(error,capacity,"Sparkle host bundle identity conflicts with compiled policy");return NULL;}
        if (![[bundle objectForInfoDictionaryKey:@"SUPublicEDKey"] isEqual:expected] || ![[bundle objectForInfoDictionaryKey:@"SUVerifyUpdateBeforeExtraction"] boolValue] || ![[bundle objectForInfoDictionaryKey:@"SURequireSignedFeed"] boolValue] || [[bundle objectForInfoDictionaryKey:@"SUAllowsAutomaticUpdates"] boolValue]) {
            snprintf(error,capacity,"Sparkle bundle verification policy conflicts with compiled policy");return NULL;
        }
        RakeSparkleBridge *bridge=[RakeSparkleBridge new];
        bridge.feed=[NSString stringWithUTF8String:feed];bridge.notify=notify;bridge.context=context;
        bridge.driver=[RakeSparkleDriver new];bridge.driver.owner=bridge;
        bridge.driver.standard=[[SPUStandardUserDriver alloc] initWithHostBundle:bundle delegate:nil];
        bridge.updater=[[SPUUpdater alloc] initWithHostBundle:bundle applicationBundle:bundle userDriver:bridge.driver delegate:bridge];
        [bridge.updater clearFeedURLFromUserDefaults];
        bridge.updater.automaticallyChecksForUpdates=NO;
        bridge.updater.automaticallyDownloadsUpdates=NO;
        bridge.updater.sendsSystemProfile=NO;
        NSError *failure=nil;
        if (![bridge.updater startUpdater:&failure]) {snprintf(error,capacity,"%s",failure.localizedDescription.UTF8String);bridge.notify=NULL;bridge.context=NULL;return NULL;}
        return (__bridge_retained void *)bridge;
    } @catch (NSException *exception) {snprintf(error,capacity,"%s",exception.reason.UTF8String);return NULL;}
}
void rake_sparkle_check(void *handle) {RakeSparkleBridge *bridge=(__bridge RakeSparkleBridge *)handle;[bridge.updater checkForUpdates];}
bool rake_sparkle_pending(void *handle) {RakeSparkleBridge *bridge=(__bridge RakeSparkleBridge *)handle;return bridge.pending && !bridge.taken;}
bool rake_sparkle_take(void *handle) {RakeSparkleBridge *bridge=(__bridge RakeSparkleBridge *)handle;if (!bridge.pending || bridge.taken) return false;bridge.taken=YES;return true;}
bool rake_sparkle_reply(void *handle,bool approved) {
    RakeSparkleBridge *bridge=(__bridge RakeSparkleBridge *)handle;
    if (!bridge.pending) return false;
    void (^resume)(BOOL)=bridge.pending;bridge.pending=nil;bridge.taken=NO;bridge.authorized=approved;
    resume(approved);return true;
}
void rake_sparkle_cancel(void *handle) {RakeSparkleBridge *bridge=(__bridge RakeSparkleBridge *)handle;void (^cancel)(void)=bridge.cancelDownload;bridge.cancelDownload=nil;if (cancel) cancel();rake_sparkle_reply(handle,false);[bridge.choice finish:SPUUserUpdateChoiceSkip];bridge.choice=nil;}
void rake_sparkle_status(void *handle,uint32_t *stage,uint64_t *received,uint64_t *total,double *extraction,char *error,size_t capacity) {
    RakeSparkleBridge *bridge=(__bridge RakeSparkleBridge *)handle;
    *stage=bridge.stage;*received=bridge.received;*total=bridge.total;*extraction=bridge.extraction;
    snprintf(error,capacity,"%s",bridge.lastError ? bridge.lastError.UTF8String : "");
}
void rake_sparkle_destroy(void *handle) {
    RakeSparkleBridge *bridge=(__bridge_transfer RakeSparkleBridge *)handle;
    bridge.notify=NULL;bridge.context=NULL;
    rake_sparkle_cancel((__bridge void *)bridge);
    bridge.updater.automaticallyChecksForUpdates=NO;
    bridge.driver.owner=nil;
}
