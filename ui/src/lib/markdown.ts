// Renderer Markdown dengan allowlist sangat sempit (PRD §14.5, mitigasi T7).
//
// Katalog adalah input yang datang dari jaringan. Meskipun kita yang
// mengendalikannya, di sisi client ia diperlakukan sebagai tidak tepercaya.
//
// Yang didukung: heading, paragraf, list, tabel, blok kode, **bold**, *italic*,
// `code`, dan gambar yang SUDAH di-cache backend.
//
// Yang TIDAK didukung, secara sengaja: link, HTML mentah, dan gambar yang
// menunjuk internet. Ketiganya adalah cara katalog yang di-tamper memicu
// request jaringan dari dalam WebView atau menjalankan skrip.

const ESCAPES: Record<string, string> = {
  "&": "&amp;",
  "<": "&lt;",
  ">": "&gt;",
  '"': "&quot;",
  "'": "&#39;",
};

/** Escape dulu, format kemudian. Urutan ini yang membuat HTML mentah mustahil. */
export function escapeHtml(text: string): string {
  return text.replace(/[&<>"']/g, (c) => ESCAPES[c]);
}

const IMAGE_PATTERN = /!\[([^\]]*)\]\(([^)\s]+)[^)]*\)/g;

/** Ekstrak URL gambar dari Markdown, untuk diserahkan ke backend. */
export function extractImageUrls(source: string): string[] {
  const urls = new Set<string>();
  for (const match of source.matchAll(IMAGE_PATTERN)) {
    if (match[2]) urls.add(match[2]);
  }
  return [...urls];
}

function inline(text: string): string {
  return escapeHtml(text)
    .replace(/`([^`]+)`/g, "<code>$1</code>")
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/(^|[^*])\*([^*]+)\*/g, "$1<em>$2</em>");
}

/** `| --- | :--: |` — baris pemisah yang menandai baris sebelumnya sebagai header. */
function isTableSeparator(line: string): boolean {
  const trimmed = line.trim();
  if (!trimmed.startsWith("|")) return false;
  return /^\|(\s*:?-{2,}:?\s*\|)+$/.test(trimmed);
}

function splitRow(line: string): string[] {
  return line
    .trim()
    .replace(/^\||\|$/g, "")
    .split("|")
    .map((c) => c.trim());
}

/** README bisa jauh lebih panjang daripada deskripsi kartu. */
const MAX_INPUT = 80_000;

/**
 * @param images Peta `URL asal → path lokal` dari backend. Gambar yang TIDAK
 *   ada di peta ini dirender sebagai teks alt, tidak pernah sebagai `<img>`
 *   yang menunjuk internet — CSP memblokirnya, dan itu memang disengaja.
 *   Peta ini satu-satunya jalan sebuah gambar tampil.
 */
export function renderMarkdown(
  source: string,
  images: Record<string, string> = {}
): string {
  if (!source) return "";

  // Batas panjang: katalog yang mengirim string raksasa tidak boleh membekukan
  // UI hanya karena kita mem-parsing-nya.
  let text = source.length > MAX_INPUT ? source.slice(0, MAX_INPUT) + "…" : source;

  // Gambar diganti token sebelum escaping, lalu dikembalikan setelah render.
  // Kalau tidak, `&` di URL menjadi `&amp;` dan pencarian di peta gagal.
  // Penanda memakai karakter yang tidak muncul di teks nyata dan tidak
  // disentuh `escapeHtml`.
  const tokens: { alt: string; url: string }[] = [];
  const OPEN = "␂IMG";
  const CLOSE = "␃";
  text = text.replace(IMAGE_PATTERN, (_match, alt: string, url: string) => {
    tokens.push({ alt: alt ?? "", url });
    return `\n${OPEN}${tokens.length - 1}${CLOSE}\n`;
  });

  const lines = text.split(/\r?\n/);
  const html: string[] = [];
  let listOpen = false;
  let paragraph: string[] = [];
  let index = 0;

  const flushParagraph = () => {
    if (paragraph.length) {
      html.push(`<p>${inline(paragraph.join(" "))}</p>`);
      paragraph = [];
    }
  };
  const closeList = () => {
    if (listOpen) {
      html.push("</ul>");
      listOpen = false;
    }
  };

  while (index < lines.length) {
    const line = lines[index].trimEnd();
    index += 1;

    // Blok kode berpagar. Harus diperiksa SEBELUM apa pun yang lain: isinya
    // teks mentah, dan aturan Markdown mana pun tidak berlaku di dalamnya.
    // Tanpa ini, diagram ASCII di README diratakan menjadi satu paragraf —
    // baris digabung dengan spasi dan strukturnya hilang sepenuhnya.
    const fence = /^\s*(```|~~~)/.exec(line);
    if (fence) {
      flushParagraph();
      closeList();

      const marker = fence[1];
      const body: string[] = [];
      while (index < lines.length) {
        const next = lines[index];
        index += 1;
        if (next.trimStart().startsWith(marker)) break;
        body.push(next);
      }
      html.push(`<pre class="md-code"><code>${escapeHtml(body.join("\n"))}</code></pre>`);
      continue;
    }

    if (!line.trim()) {
      flushParagraph();
      closeList();
      continue;
    }

    // Garis pembatas. Diperiksa sebelum list, karena `---` juga cocok dengan
    // pola bullet `-` dan akan menjadi butir kosong kalau kecolongan.
    if (/^\s*(-{3,}|\*{3,}|_{3,})\s*$/.test(line)) {
      flushParagraph();
      closeList();
      html.push("<hr class='md-rule' />");
      continue;
    }

    // Gambar berdiri sendiri sebagai blok.
    if (line.trim().startsWith(OPEN)) {
      flushParagraph();
      closeList();
      html.push(line.trim());
      continue;
    }

    // Tabel: baris header, baris pemisah `|---|---|`, lalu isinya. README
    // sungguhan penuh tabel, dan tanpa dukungan ini ia tampil sebagai deretan
    // tanda pipa yang tidak terbaca.
    if (line.trimStart().startsWith("|") && isTableSeparator(lines[index] ?? "")) {
      flushParagraph();
      closeList();
      const header = splitRow(line);
      index += 1; // lewati baris pemisah

      const rows: string[][] = [];
      while (index < lines.length && lines[index].trimStart().startsWith("|")) {
        rows.push(splitRow(lines[index]));
        index += 1;
      }

      html.push("<div class='md-table-wrap'><table><thead><tr>");
      for (const cell of header) html.push(`<th>${inline(cell)}</th>`);
      html.push("</tr></thead><tbody>");
      for (const row of rows) {
        html.push("<tr>");
        for (let i = 0; i < header.length; i += 1) {
          html.push(`<td>${inline(row[i] ?? "")}</td>`);
        }
        html.push("</tr>");
      }
      html.push("</tbody></table></div>");
      continue;
    }

    const heading = /^(#{1,6})\s+(.*)$/.exec(line);
    if (heading) {
      flushParagraph();
      closeList();
      // Heading dipetakan turun dua tingkat: `###` di changelog tidak boleh
      // bersaing dengan struktur heading halaman.
      const level = Math.min(6, heading[1].length + 2);
      html.push(`<h${level}>${inline(heading[2])}</h${level}>`);
      continue;
    }

    const bullet = /^\s*[-*+]\s+(.*)$/.exec(line);
    if (bullet) {
      flushParagraph();
      if (!listOpen) {
        html.push("<ul>");
        listOpen = true;
      }
      html.push(`<li>${inline(bullet[1])}</li>`);
      continue;
    }

    closeList();
    paragraph.push(line.trim());
  }

  flushParagraph();
  closeList();

  // Kembalikan token menjadi gambar — hanya untuk URL yang backend berhasil
  // ambil dan validasi. Sisanya menjadi teks alt.
  const pattern = new RegExp(`${OPEN}(\\d+)${CLOSE}`, "g");
  return html.join("").replace(pattern, (_match, raw: string) => {
    const token = tokens[Number(raw)];
    if (!token) return "";
    const local = images[token.url];
    if (!local) {
      return token.alt ? `<p class="md-img-alt">${escapeHtml(token.alt)}</p>` : "";
    }
    return `<img class="md-img" src="${escapeHtml(local)}" alt="${escapeHtml(token.alt)}" loading="lazy" />`;
  });
}
