use rzn_plan::dom_analyzer::DomAnalyzer;

fn main() {
    // Sample HTML with lots of script tags and noise (like what you showed)
    let html = r#"
<!DOCTYPE html>
<html>
<head>
    <title>Test Page</title>
    <script>
        // Obfuscated Google code
        ;oSUNyd:fTfGO,fTfGO;oUlnpc:RagDlc;okUaUd:wItadb;pKJiXd:VCenhc;pNsl2d:j9Yuyc;
        pXdRYb:JKoKVe;pj82le:ww04Df;qZx2Fc:j0xrE;qaS3gd:yiLg6e;qafBPd:sgY6Zb;
        qavrXe:zQzcXe;qddgKe:d7YSfd,x4FYXe;rQSrae:C6D5Fc;rdexKf:FEkKD;rmWaj:PMS6Sd;
        window.google = {
            kEI: 'abc123',
            kEXPI: '0,1302536,56873,6059,206,4804,2316,383,246,5,1354,4013,1238,1122515',
        };
    </script>
    <script src="/xjs/_/js/k=xjs.hd.en_GB.2_OC1eW-kJ8.2018.O/am=..."></script>
</head>
<body>
    <div id="searchform">
        <form action="/search" method="GET">
            <textarea name="q" aria-label="Search" class="gLFyf"></textarea>
            <button type="submit" aria-label="Google Search">Google Search</button>
        </form>
    </div>
    <div id="search" class="results">
        <div class="g" data-testid="result">
            <h3 class="LC20lb">Rust Programming Language</h3>
            <a href="https://rust-lang.org">rust-lang.org</a>
            <span>A language empowering everyone to build reliable software.</span>
        </div>
        <div class="g" data-testid="result">
            <h3 class="LC20lb">Rust Tutorial</h3>
            <a href="https://example.com">example.com</a>
            <span>Learn Rust programming from scratch.</span>
        </div>
    </div>
    <script>
        // More inline scripts
        (function(){window.jsl=window.jsl||{};window.jsl.dh=function(a,b){try{var c=document.getElementById(a);if(c){c.innerHTML=b;}}catch(e){}};})();
    </script>
</body>
</html>
    "#;

    println!("[SEARCH] Testing DOM Reduction");
    println!("========================\n");
    println!("Original HTML size: {} chars", html.len());
    println!("Sample of original (first 200 chars):");
    println!("{}", &html[..200.min(html.len())]);
    println!("\n--- Running DOM Reduction ---\n");

    let analyzer = DomAnalyzer::new(30_000);
    match analyzer.reduce_html(html) {
        Ok(reduced) => {
            println!("[OK] Reduction successful!");
            println!("Reduced DOM size: {} chars", reduced.len());
            println!(
                "Reduction ratio: {:.1}%",
                (reduced.len() as f64 / html.len() as f64) * 100.0
            );
            println!("\n--- REDUCED OUTPUT (THIS IS WHAT GETS SENT TO LLM) ---\n");
            println!("{}", reduced);
        }
        Err(e) => {
            println!("[ERROR] Error reducing HTML: {:?}", e);
        }
    }
}
