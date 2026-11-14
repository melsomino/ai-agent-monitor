#!/usr/bin/env swift
// Development-only rasterization of the prototype's shared SVG mesh.
// The production AppKit app uses the PNG; WebKit is a development-only tool.
import AppKit
import WebKit

let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
let source = try String(contentsOf: root.appendingPathComponent("src/web/index.html"), encoding: .utf8)
let pattern = #"<svg class="aam-mesh"[\s\S]*?</svg>"#
let expression = try NSRegularExpression(pattern: pattern)
guard let match = expression.firstMatch(in: source, range: NSRange(source.startIndex..., in: source)),
      let range = Range(match.range, in: source) else { fatalError("Prototype mesh not found") }
var svg = String(source[range]).replacingOccurrences(of: "${index}", with: "0")
svg = svg.replacingOccurrences(of: "viewBox=\"0 0 460 1000\"", with: "viewBox=\"0 -128 460 1256\"")
// Grain is appearance-independent and is not baked into overlapping tiles.
svg = svg.replacingOccurrences(of: #"<rect width="460" height="1000" filter="url(#aam-mesh-grain-0)" opacity=".018"/>"#, with: "")
let css = try String(contentsOf: root.appendingPathComponent("src/web/monitor.css"), encoding: .utf8)
let blurExpression = try NSRegularExpression(pattern: #"filter: blur\((\d+)px\); transform: scale"#)
guard let blurMatch = blurExpression.firstMatch(in:css,range:NSRange(css.startIndex...,in:css)),
      let blurRange = Range(blurMatch.range(at:1),in:css) else { fatalError("Mesh blur not found") }
let html = """
<html><head><style>html,body{margin:0;background:transparent;width:460px;height:1256px;overflow:hidden}svg{display:block;width:460px;height:1256px;filter:blur(\(css[blurRange])px)}</style></head><body>\(svg)</body></html>
"""
let app=NSApplication.shared
app.setActivationPolicy(.prohibited)
let configuration=WKWebViewConfiguration()
configuration.websiteDataStore = .nonPersistent()
let web=WKWebView(frame:NSRect(x:0,y:0,width:460,height:1256),configuration:configuration)
web.underPageBackgroundColor = .clear
let window=NSWindow(contentRect:web.frame,styleMask:.borderless,backing:.buffered,defer:false)
window.isOpaque=false
window.backgroundColor = .clear
window.contentView=web
final class Capture:NSObject,WKNavigationDelegate {
    var black:NSBitmapImageRep?
    func webView(_ webView:WKWebView,didFinish navigation:WKNavigation!) { capture(webView,white:false) }
    func capture(_ web:WKWebView,white:Bool) {
        let color=white ? "#fff" : "#000"
        web.evaluateJavaScript("document.documentElement.style.background='\(color)';document.body.style.background='\(color)'") { _,error in
            if let error { print(error);exit(1) }
            DispatchQueue.main.asyncAfter(deadline:.now()+0.1) {
                let config=WKSnapshotConfiguration()
                config.rect=web.bounds
                config.snapshotWidth=460
                web.takeSnapshot(with:config) { image,error in
                    guard let tiff=image?.tiffRepresentation,let bitmap=NSBitmapImageRep(data:tiff) else { print("Mesh render failed: \(String(describing:error))");exit(1) }
                    if !white { self.black=bitmap; self.capture(web,white:true);return }
                    self.save(black:self.black!,white:bitmap)
                }
            }
        }
    }
    func save(black:NSBitmapImageRep,white:NSBitmapImageRep) {
        // Recover transparent SVG coverage from two opaque WebKit snapshots.
        // This avoids private WKWebView background APIs and preserves soft edges.
        precondition(black.bitsPerSample==8 && white.bitsPerSample==8 && black.samplesPerPixel==4 && white.samplesPerPixel==4)
        let result=NSBitmapImageRep(bitmapDataPlanes:nil,pixelsWide:black.pixelsWide,pixelsHigh:black.pixelsHigh,bitsPerSample:8,samplesPerPixel:4,hasAlpha:true,isPlanar:false,colorSpaceName:black.colorSpaceName,bitmapFormat:.alphaNonpremultiplied,bytesPerRow:0,bitsPerPixel:0)!
        if let profile=black.value(forProperty:.colorSyncProfileData) { result.setProperty(.colorSyncProfileData,withValue:profile) }
        let bp=black.bitmapData!,wp=white.bitmapData!,out=result.bitmapData!
        let bo=black.bitmapFormat.contains(.alphaFirst) ? 1 : 0
        let wo=white.bitmapFormat.contains(.alphaFirst) ? 1 : 0
        for y in 0..<black.pixelsHigh {
            for x in 0..<black.pixelsWide {
                let bi=y*black.bytesPerRow+x*4+bo,wi=y*white.bytesPerRow+x*4+wo,oi=y*result.bytesPerRow+x*4
                let differences=(0..<3).map { Int(wp[wi+$0])-Int(bp[bi+$0]) }.sorted()
                let alpha=max(0,min(255,255-differences[1]))
                out[oi+3]=UInt8(alpha)
                for channel in 0..<3 { out[oi+channel]=alpha==0 ? 0 : UInt8(max(0,min(255,Int(bp[bi+channel])*255/alpha))) }
            }
        }
        guard let png=result.representation(using:.png,properties:[:]) else { fatalError("PNG encoding failed") }
        do { try png.write(to:root.appendingPathComponent("assets/monitor-mesh.png")) } catch { print(error);exit(1) }
        print("Generated transparent assets/monitor-mesh.png (\(result.pixelsWide)×\(result.pixelsHigh))")
        exit(0)
    }
}
let capture=Capture()
web.navigationDelegate=capture
web.loadHTMLString(html,baseURL:nil)
DispatchQueue.main.asyncAfter(deadline:.now()+20) { print("Mesh render timed out");exit(1) }
app.run()
