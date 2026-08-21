<div align="center">

# ctxmenu

**Take your right-click menu back.**

See every entry, where it lives in the registry and which program put it there. <br>
Hide it, sort it, delete it, or build your own — and every change is backed up before it happens.

[**⬇ Download ctxmenu.exe**](https://github.com/corgan2222/context-manager/releases/latest) · [**Documentation**](https://corgan2222.github.io/context-manager/)

*Windows 10 and 11 · 64-bit · No installer. No runtime library. No background service.*

</div>

[![ctxmenu](https://raw.githubusercontent.com/corgan2222/context-manager/refs/heads/main/docs/images/ctxmenu-header.gif)](https://corgan2222.github.io/context-manager)

## Features

|     |     |
| --- | --- |
| ✅ Free and open source | ✅ Dark and light mode |
| ✅ MIT licence | ✅ German and English, switched at runtime |
| ✅ Windows 11 ready¹ | ✅ Add entries by drag and drop |
| ✅ Works without administrator rights² | ✅ Search across names, paths, commands and CLSIDs |
| ✅ Four switches instead of a row of verbs | ✅ Finds entries whose program is gone |
| ✅ Automatic backups before every change | ✅ Built-in services from any OpenAPI address |
| ✅ Updates itself, signature checked first | ✅ Built-in help, with working command lines |
| ✅ A favourites list you place from | ✅ Built in Rust: one file, no runtime library |

<sup>1</sup> The entries of the Windows 11 menu are listed, hidden and created. Their order up there belongs to Explorer, and the commands it builds in itself carry no registration to reach.
<sup>2</sup> Your own entries always go to `HKCU`. Changing an entry that lives under `HKLM` needs elevation, and it is asked for that one step only.

## Services it already knows

Pick one in the favourites tab and every field is filled in but the key. The
list is a plain [JSON file](ctxmenu/templates.json) — adding a service is a
pull request on a text file, not on Rust.

| | | |
|---|---|---|
| **Share images** | <a href="https://imgur.com"><img src="https://corgan2222.github.io/context-manager/icons/imgur.png" height="20" alt="Imgur"></a> [Imgur](https://imgur.com) | Uploads an image and hands back its address |
|  | <a href="https://imgbb.com"><img src="https://corgan2222.github.io/context-manager/icons/imgbb.png" height="20" alt="ImgBB"></a> [ImgBB](https://imgbb.com) | Uploads an image and hands back its address |
|  | <a href="https://freeimage.host"><img src="https://corgan2222.github.io/context-manager/icons/freeimage-host.png" height="20" alt="Freeimage.host"></a> [Freeimage.host](https://freeimage.host) | Uploads an image and hands back its address |
| **Share files** | <a href="https://catbox.moe"><img src="https://corgan2222.github.io/context-manager/icons/catbox.png" height="20" alt="Catbox"></a> [Catbox](https://catbox.moe) | Puts any file up for good, up to 200 MB |
|  | <a href="https://gofile.io"><img src="https://corgan2222.github.io/context-manager/icons/gofile.png" height="20" alt="Gofile"></a> [Gofile](https://gofile.io) | Puts any file up, no account needed |
| **Edit images** | <a href="https://www.remove.bg"><img src="https://corgan2222.github.io/context-manager/icons/removebg.png" height="20" alt="remove.bg"></a> [remove.bg](https://www.remove.bg) | Takes the background out of a photo |
|  | <a href="https://www.photoroom.com"><img src="https://corgan2222.github.io/context-manager/icons/photoroom.png" height="20" alt="PhotoRoom"></a> [PhotoRoom](https://www.photoroom.com) | Takes the background out of a photo |
|  | <a href="https://stability.ai"><img src="https://corgan2222.github.io/context-manager/icons/stability-ai.png" height="20" alt="Stability AI"></a> [Stability AI](https://stability.ai) | Enlarges an image four times over |
|  | <a href="https://tinypng.com"><img src="https://corgan2222.github.io/context-manager/icons/tinypng.png" height="20" alt="TinyPNG"></a> [TinyPNG](https://tinypng.com) | Makes a PNG or JPEG smaller |
| **Documents** | <a href="https://www.stirlingpdf.com"><img src="https://corgan2222.github.io/context-manager/icons/stirling-pdf.png" height="20" alt="Stirling-PDF"></a> [Stirling-PDF](https://www.stirlingpdf.com) | Shrinks a PDF, on a server of your own |
|  | <a href="https://gotenberg.dev"><img src="https://corgan2222.github.io/context-manager/icons/gotenberg.png" height="20" alt="Gotenberg"></a> [Gotenberg](https://gotenberg.dev) | Turns an Office document into a PDF |
|  | <a href="https://www.nutrient.io"><img src="https://corgan2222.github.io/context-manager/icons/nutrient.png" height="20" alt="Nutrient DWS"></a> [Nutrient DWS](https://www.nutrient.io) | Turns a file into a PDF |
| **Share text** | <a href="https://paste.rs"><img src="https://corgan2222.github.io/context-manager/icons/paste-rs.png" height="20" alt="paste.rs"></a> [paste.rs](https://paste.rs) | Shares a text file and hands back its address |
|  | <a href="https://bpa.st"><img src="https://corgan2222.github.io/context-manager/icons/bpast.png" height="20" alt="bpa.st"></a> [bpa.st](https://bpa.st) | Shares a text file; the address is in the report |
| **Development** | <a href="https://about.gitea.com"><img src="https://corgan2222.github.io/context-manager/icons/gitea.png" height="20" alt="Gitea"></a> [Gitea](https://about.gitea.com) | Attaches the file to an issue |
|  | <a href="https://forgejo.org"><img src="https://corgan2222.github.io/context-manager/icons/forgejo.png" height="20" alt="Forgejo"></a> [Forgejo](https://forgejo.org) | Attaches the file to an issue |
|  | <a href="https://zipline.diced.sh"><img src="https://corgan2222.github.io/context-manager/icons/zipline.png" height="20" alt="Zipline"></a> [Zipline](https://zipline.diced.sh) | Puts the file on a host of your own |
| **Storage of your own** | <a href="https://nextcloud.com"><img src="https://corgan2222.github.io/context-manager/icons/nextcloud.png" height="20" alt="Nextcloud"></a> [Nextcloud](https://nextcloud.com) | Puts the file into your own Nextcloud |
|  | <a href="https://bunny.net"><img src="https://corgan2222.github.io/context-manager/icons/bunny.png" height="20" alt="Bunny Storage"></a> [Bunny Storage](https://bunny.net) | Puts the file into a storage zone |
|  | <a href="https://min.io"><img src="https://corgan2222.github.io/context-manager/icons/minio.png" height="20" alt="MinIO"></a> [MinIO](https://min.io) | Puts the file into an open bucket |
| **Check a file** | <a href="https://www.virustotal.com"><img src="https://corgan2222.github.io/context-manager/icons/virustotal.png" height="20" alt="VirusTotal"></a> [VirusTotal](https://www.virustotal.com) | Has the file checked; the report opens |


Anything not on this list is a form to fill in once, and the
[formats page](https://corgan2222.github.io/context-manager/docs/formats)
says which shapes of API it can speak to. A service that describes itself
through OpenAPI needs no form at all: paste the address and tick the tools.

## What it looks like

[![ctxmenu](https://raw.githubusercontent.com/corgan2222/context-manager/refs/heads/main/docs/images/01-overview_en.web.png)](https://corgan2222.github.io/context-manager)

[![ctxmenu](https://raw.githubusercontent.com/corgan2222/context-manager/refs/heads/main/docs/images/10-many-entries_en.web.png)](https://corgan2222.github.io/context-manager)

## License

[MIT](LICENSE)
