"""Render the server's sign-in page (backend crates/rzn-tenancy/src/http/mcp_oauth.rs) to static
HTML for screenshots, without compiling the server: the templates are read out of the Rust source.

    python3 render_signin.py <path to mcp_oauth.rs> <out dir>

Writes signin-email.html, signin-code.html, signin-code-wrong.html and signin-refused.html."""
import re, sys
from pathlib import Path

src = Path(sys.argv[1]).read_text()
out = Path(sys.argv[2]); out.mkdir(parents=True, exist_ok=True)

def raw(name):
    return re.search(rf'const {name}: &str = r#*"(.*?)"#*;', src, re.S).group(1)

def fmt(template, **values):
    # Rust format strings: {{ and }} are literal braces; {name} and {0} are arguments.
    template = template.replace('{{', '\0').replace('}}', '\1')
    template = re.sub(r'\{(\w+)\}', lambda m: values[m.group(1)], template)
    return template.replace('\0', '{').replace('\1', '}')

shell = re.search(r'fn html\(.*?Html\(format!\(\s*r#"(.*?)"#', src, re.S).group(1)
page = src[src.index('fn page('):]
email_step = re.search(r'None => format!\(\s*r#"(.*?)"#', page, re.S).group(1)
code_step = re.search(r'Some\(email\) => format!\(\s*r#"(.*?)"#', page, re.S).group(1)
refusal = re.search(r'fn refusal.*?r#"(.*?)"#', src, re.S).group(1)

hidden = ''.join(f'<input type="hidden" name="{k}" value="{v}">' for k, v in [
    ('response_type', 'code'), ('client_id', 'rzn-browser'), ('redirect_uri', 'https://bogjdnehdficgkhklinmnbgiiofbamji.chromiumapp.org/'),
    ('code_challenge', 'x' * 43), ('code_challenge_method', 'S256'), ('state', 'st'), ('scope', ''), ('resource', '')])
client = 'RZN Browser'
note = lambda text: f'<p class="note">{text}</p>' if text else ''
wrap = lambda body: fmt(shell, style=raw('STYLE'), mark=raw('MARK'), body=body)

(out / 'signin-email.html').write_text(wrap(fmt(email_step, client=client, note='', hidden=hidden)))
(out / 'signin-code.html').write_text(wrap(fmt(code_step, client=client, note='', hidden=hidden, **{'0': 'sam@example.com'})))
(out / 'signin-code-wrong.html').write_text(wrap(fmt(code_step, client=client, note=note('That code did not work. Check it, or start again for a new one.'), hidden=hidden, **{'0': 'sam@example.com'})))
(out / 'signin-refused.html').write_text(wrap(refusal.replace('{}', "This assistant&#39;s return address is not one it registered.")))
