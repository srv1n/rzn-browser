# Legal source captures for the rzn backend

Requested 2026-10-15 by the backend corpus work. Same pattern as `workflows/sci/`: a browser flow searches a site that blocks plain HTTP, solves its CAPTCHA with a cheap model, and writes the result rows as JSONL. The backend mirrors those rows into R2 with `scripts/producers/sources/rows.py`. No backend code is needed per site.

## Contract: what each capture writes

One JSON object per line, one line per document:

```json
{"id": "<stable id from the site>", "pdf_url": "<PDF URL a plain GET can fetch>", "metadata": {"title": "...", "date": "YYYY-MM-DD", "case_number": "...", "parties": "...", "bench": "...", "members": "...", "document_type": "order|judgment|decision"}}
```

- `id` must be stable and come from the site: case or appeal number plus bench plus order date, or the site's own order id. Use the PDF URL only when nothing better exists.
- If the PDF link only works inside the browser session, download the PDF and write `"pdf_file": "/absolute/path.pdf"` instead of `pdf_url`.
- `metadata` holds only what the site shows, in the site's own spelling. Omit a field the site does not show. Never fill a default.
- Output directory: `~/rzn-legal-captures/<source>/rows.jsonl`, plus `pdfs/` when needed. Use the source names in the table below.

The backend runs: `python3 scripts/producers/sources/rows.py <source> --rows ~/rzn-legal-captures/<source>/rows.jsonl`.

## Acceptance per source

1. A workflow pair like `sci/judgements-open` and `sci/judgements-by-date`: open the search, return the CAPTCHA image, then type the answer and read every result page.
2. A driver like `scripts/sci/sci-listing.sh` with `window <from> <to>`, `backfill` and `weekly` modes, using the same CAPTCHA solver.
3. Proof: one window that yields at least 5 rows whose PDFs open and whose first page matches the row's metadata.
4. A "Step 0 findings" section in the workflow README: maximum date range, paging, CAPTCHA kind, PDF link shape, columns, and how far back the archive goes.

## Sources, in priority order

Field names and routes below come from plain-HTTP probes on 2026-10-15. Anything past the CAPTCHA was not observed, so verify it in the browser.

### 1. `itat`: Income Tax Appellate Tribunal

- Start: https://itat.gov.in/judicial/tribunal_orders. HTTPS failed certificate validation in Python; the browser should be fine.
- A six-character image CAPTCHA sits above the tabs (image `/captcha/show`). The page checks it through `/Ajax/checkCaptcha` before it searches.
- Use "Search by Order Date": Bench (`bench_name_2`), Appeal Type (`app_type_2`), Date of Order (`order_date`).
- The order opens in an iframe (`detailsFrame`). Capture the final PDF URL.

### 2. `nclt`: National Company Law Tribunal

- Start: https://nclt.gov.in/order-date-wise.
- Fields: Bench (`bench`, for example Principal Bench `delhi_1`), From Date (`start_date`), To Date (`end_date`), CAPTCHA digits (`captcha_input`) below the dates. The form POSTs to `/order-date-wise-search`.
- Result URLs carry base64 parameters, for example `/order-date-wise-search?bench=bXVtYmFp&end_date=MTAvMDYvMjAyNg%3D%3D&page=10&start_date=MDkvMDYvMjAyNg%3D%3D`.
- Rows show filing number, case number, parties, listing date, court and status. Capture the order download link.

### 3. `cestat`: Customs, Excise and Service Tax Appellate Tribunal

- Start: https://cestat.gov.in/final-order-status. Use the "ORDER DATE WISE" tab: Bench (`bench`), From (`from`), To (`to`) as DD-MM-YYYY.
- An image CAPTCHA (`/captcha?rand=...`, field `captcha_code`) and a CSRF field. The form POSTs to `/order-status-web` with `order_type=F`.
- Known PDF shape: `https://cestat.gov.in/weborders/file/<bench>/<numeric-id>`.

### 4. `ncdrc`: National Consumer Disputes Redressal Commission (state commissions after)

- Start: https://e-jagriti.gov.in/. The site is JavaScript only; plain HTTP returns an empty app shell.
- Judgments: choose JUDGEMENT, then NCDRC. Fallback: https://e-jagriti.gov.in/advance-case-search with Commission NCDRC, a filing-date range, "View As Table", and the CAPTCHA above SUBMIT.
- Do NOT store the filing-help PDFs. Only judgments and orders.

### 5. `drt`: Debts Recovery Tribunals and Appellate Tribunals

- Start: https://drt.gov.in/ (a React app).
- DRAT orders by date: choose DRAT, order type, From Date and To Date, then solve the CAPTCHA beside the search button. The app calls `getDratOrderJudgementReportFromToDate` with `schemeNameDrtId`, `dratDailyFinalOrderId`, `fromDate`, `toDate`.
- DRT final orders search by case number or diary number only, with a CAPTCHA.
- The result table's PDF icon links to the response field `orderPdf`.

### 6. `cat`: Central Administrative Tribunal

- Start: https://cgat.gov.in/. Choose a bench, open "Oral & Final Order", then the date-wise final-order tab with From Date and To Date.
- The app calls `getCatFinalOrderReportDatewise` with `catschemaId`, `fromDate`, `toDate`. No CAPTCHA was seen in the bundle, so check the live page.
- Rows show diary number, case number, applicant v. respondent, order date and upload date. The PDF link is the field `dailyOrderPdf`.

### 7. `cic`: Central Information Commission

- Start: https://cic.gov.in/decision, then "Decision after 17th August 2026", which opens https://dss.cic.gov.in/citizen-service?tab=decision.
- Python failed certificate validation on this host. That is a transport problem, not a CAPTCHA, so a browser may not even meet a CAPTCHA here.
- Older decisions live on separate pages for 11 Sep 2016–16 Aug 2026 and before 10 Sep 2016.

### 8. `privycouncil`: Privy Council judgments on Indian appeals

- Start: https://www.bailii.org/uk/cases/UKPC/. BAILII serves an Anubis proof-of-work page ("Making sure you're not a bot!") to scripts. A normal browser passes it.
- Keep only appeals from India, judged by the origin stated in the judgment, not by party names.
- BAILII judgments are often HTML, not PDF. If so, save the page as a PDF and write `pdf_file`.

### 9. `rbi`: RBI circulars and master directions (PDF download only)

- The backend lists these already (`scripts/producers/sources/rbi.py`). Only the download is blocked.
- Listings: https://www.rbi.org.in/Scripts/NotificationUser.aspx (archive year, then All Months) and https://www.rbi.org.in/Scripts/BS_ViewMasterDirections.aspx.
- Each PDF link on `rbidocs.rbi.org.in` returns an HTML CAPTCHA page ("What code is in the image?", input `answer` id `ans`, submit id `jar`) instead of the PDF.
- Find out whether one solved CAPTCHA unlocks later downloads in the same session. If it does, the cheapest flow is: solve once, then download a batch and write `pdf_file` rows.

### 10. `sebi`: SEBI orders and circulars, pages after the first

- The backend mirrors page 1 of each listing. Paging POSTs to `/sebiweb/ajax/home/getnewslistinfo.jsp`, which returns HTTP 530 BLOCKED to scripts. No CAPTCHA was seen.
- Orders: https://www.sebi.gov.in/sebiweb/home/HomeAction.do?doListing=yes&sid=2&ssid=9&smid=0 (31,003 listed). Circulars: the same route with `sid=1&ssid=7&smid=0` (2,806 listed).
- Rows show the date and a linked title. The detail page URL ends `_<article id>.html`; use `order:<id>` or `circular:<id>` as the row `id`. The detail page embeds the PDF as `https://www.sebi.gov.in/sebi_data/attachdocs/...pdf`, which a plain GET can fetch.

## Partial sources the backend already covers

- `nclat` (NCLAT): the backend mirrors the homepage's latest orders. Full history needs https://nclat.nic.in/display-board/judgement_details (an AJAX search with no CAPTCHA seen). This one is low priority.

Probe evidence (saved HTML and notes) lives in the backend repo under `.tusker/scratch/source-scrapers/lane1/` and `lane3/`.
