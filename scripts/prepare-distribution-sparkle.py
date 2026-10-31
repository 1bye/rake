"""Prepare an isolated signed Sparkle N/N+1 GUI acceptance project.

Run this once in an empty temporary directory, then use rake bundle for versions
1.0.0/mac-app and 1.0.1/zip. The native-tests override only permits loopback HTTP;
production policy remains HTTPS. Test keys never enter the application bundle.
See docs/distribution-native-tests.md for the native interaction assertions.
"""
import argparse,base64,json,pathlib,plistlib,subprocess,os
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output',type=pathlib.Path,required=True)
parser.add_argument('--port',type=int,default=19091)
args=parser.parse_args()
if not 1024<=args.port<=65535: parser.error('port must be 1024..65535')
root=args.output.resolve()
if root.exists() and any(root.iterdir()): parser.error("use an empty fixture directory")
repo=pathlib.Path(__file__).resolve().parents[1]
root.mkdir(parents=True,exist_ok=True)
source=root/'source';(source/'src').mkdir(parents=True,exist_ok=True)
private=root/'test-key.pem'
subprocess.run(['openssl','genpkey','-algorithm','ED25519','-out',str(private)],check=True)
os.chmod(private,0o600)
subprocess.run(['openssl','pkey','-in',str(private),'-outform','DER','-out',str(root/'test-key.der')],check=True)
subprocess.run(['openssl','pkey','-in',str(private),'-pubout','-outform','DER','-out',str(root/'test-public.der')],check=True)
key=(root/'test-key.der').read_bytes();public=(root/'test-public.der').read_bytes()
assert len(key)==48 and key[:16]==bytes.fromhex('302e020100300506032b657004220420')
assert len(public)==44 and public[:12]==bytes.fromhex('302a300506032b6570032100')
(root/'test-sparkle.key').write_bytes(base64.b64encode(key[-32:]));os.chmod(root/'test-sparkle.key',0o600)
(source/'sparkle.pub').write_bytes(base64.b64encode(public[-32:]))
(source/'test-feed-url.txt').write_text(f'http://127.0.0.1:{args.port}/appcast.xml')
(source/'Info.plist').write_bytes(plistlib.dumps({'NSAppTransportSecurity':{'NSAllowsArbitraryLoads':True}}))
(source/'build.rs').write_text('fn main(){rake_build::build().unwrap();}')
(source/'src/main.rs').write_text(r'''
use rake::prelude::*;
use rake::gpui::prelude::*;
use rake::gpui::{AnyView,Context,Window,div,rgb};
use rake::router::MountError;
#[derive(Clone,Route)] enum Destination {#[route(id="document",path="/")] Document}
struct Routes;
struct Document;
fn state()->std::path::PathBuf {std::env::temp_dir().join("rake-sparkle-native-state")}
impl Render for Document {fn render(&mut self,_window:&mut Window,_cx:&mut Context<Self>)->impl IntoElement {
    div().size_full().bg(rgb(0x18232b)).text_color(rgb(0xf0f4e8)).p_8().child(format!("Rake update test — version {}. Unsaved document: keep this work.",env!("CARGO_PKG_VERSION")))
}}
impl RouteViews<Destination> for Routes {
    fn mount(&self,_route:&Destination,page:PageContext<Destination>,_window:&mut Window,cx:&mut rake::gpui::App)->Result<AnyView,MountError> {
        let entry=page.entry();entry.set_dirty(true).map_err(|error|MountError(error.to_string()))?;
        entry.clone().on_dispose(move|context|{let entry=entry.clone();async move {
            if !context.dirty {return GuardResult::Allow;}
            GuardResult::Ask(DecisionRequest{message:"Save the document before installing the update?".into(),choices:vec![
                DecisionChoice::proceed("save","Save").prepare(move||{let entry=entry.clone();async move {
                    std::fs::write(state().join("document"),"keep this work").map_err(|error|rake::router::GuardReason(error.to_string()))?;
                    entry.set_dirty(false).map_err(|error|rake::router::GuardReason(error.to_string()))
                }}),DecisionChoice::proceed("discard","Discard"),DecisionChoice::cancel("cancel","Cancel")
            ]})
        }}).map_err(|error|MountError(error.to_string()))?;
        Ok(cx.new(|_|Document).into())
    }
}
#[rake::app]
fn application()->App<Destination> {
    std::fs::create_dir_all(state()).unwrap();
    let marker=state().join("persistent-user-data");
    if marker.exists() {assert_eq!(std::fs::read_to_string(&marker).unwrap(),"preserved data");} else {std::fs::write(&marker,"preserved data").unwrap();}
    std::fs::write(state().join(format!("{}.started",env!("CARGO_PKG_VERSION"))),"running").unwrap();
    let resources=rake::resources!().unwrap();
    let feed=std::fs::read_to_string(resources.path("test-feed-url.txt").unwrap()).unwrap();
    let updates=rake::updates::UpdateService::new(env!("RAKE_UPDATE_POLICY"),&resources).unwrap().testing_sparkle_feed(&feed).unwrap();
    App::new("rake-sparkle-acceptance").service(updates).unwrap().routes(Routes).initial(Destination::Document)
}
''')
manifest=f'''[workspace]
[package]
name="rake-sparkle-fixture"
version="1.0.0"
edition="2024"
[dependencies]
rake={{path={json.dumps(str(repo/'crates/rake'))}}}
rake-updates={{path={json.dumps(str(repo/'crates/rake-updates'))},features=["native-tests","sparkle"]}}
[build-dependencies]
rake-build={{path={json.dumps(str(repo/'crates/rake-build'))}}}
[package.metadata.rake.bundle]
schema_version=1
name="Rake Update Test"
identifier="dev.rake.sparkle.acceptance"
resources=[{{from="test-feed-url.txt",to="test-feed-url.txt"}}]
[package.metadata.rake.bundle.macos]
info_plist="Info.plist"
[package.metadata.rake.bundle.updates]
enabled=true
base_url="https://updates.example.invalid"
[package.metadata.rake.bundle.updates.public_keys]
sparkle="sparkle.pub"
'''
(source/'Cargo.toml').write_text(manifest)
print('Prepared isolated Sparkle fixture; private test signing material remains in protected temporary files')
