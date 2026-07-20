class NullTranslations:
    def __init__(self, fp=None):
        self._info = {}
        self._charset = None
        self._fallback = None
        if fp is not None:
            self._parse(fp)

    def _parse(self, fp):
        pass

    def add_fallback(self, fallback):
        if self._fallback:
            self._fallback.add_fallback(fallback)
        else:
            self._fallback = fallback

    def gettext(self, message):
        if self._fallback:
            return self._fallback.gettext(message)
        return message

    def ngettext(self, singular, plural, n):
        if self._fallback:
            return self._fallback.ngettext(singular, plural, n)
        if n == 1:
            return singular
        return plural

    def pgettext(self, context, message):
        if self._fallback:
            return self._fallback.pgettext(context, message)
        return message

    def npgettext(self, context, singular, plural, n):
        if self._fallback:
            return self._fallback.npgettext(context, singular, plural, n)
        if n == 1:
            return singular
        return plural

    def lgettext(self, message):
        return self.gettext(message)

    def lngettext(self, singular, plural, n):
        return self.ngettext(singular, plural, n)

    def info(self):
        return self._info

    def charset(self):
        return self._charset

    def install(self, names=None):
        import builtins
        builtins._ = self.gettext
        if names:
            if "gettext" in names:
                builtins.gettext = self.gettext
            if "ngettext" in names:
                builtins.ngettext = self.ngettext


class GNUTranslations(NullTranslations):
    LE_MAGIC = 0x950412DE
    BE_MAGIC = 0xDE120495
    CONTEXT = "\x04"

    def _parse(self, fp):
        data = fp.read()
        self._catalog = {}
        self._plural = lambda n: 0 if n == 1 else 1
        magic = int.from_bytes(data[0:4], "little")
        if magic == self.LE_MAGIC:
            endian = "little"
        elif magic == self.BE_MAGIC:
            endian = "big"
        else:
            raise OSError("Bad magic number in .mo file")

        def u32(offset):
            return int.from_bytes(data[offset:offset + 4], endian)

        msgcount = u32(8)
        masteridx = u32(12)
        transidx = u32(16)
        for i in range(msgcount):
            mlen = u32(masteridx)
            moff = u32(masteridx + 4)
            tlen = u32(transidx)
            toff = u32(transidx + 4)
            msg = data[moff:moff + mlen]
            tmsg = data[toff:toff + tlen]
            if mlen == 0:
                last_key = None
                for line in tmsg.decode("utf-8").splitlines():
                    if ":" in line:
                        key, _, value = line.partition(":")
                        key = key.strip().lower()
                        value = value.strip()
                        self._info[key] = value
                        last_key = key
                    elif last_key and line.strip():
                        self._info[last_key] += "\n" + line
            if b"\x00" in msg:
                msgid1, msgid2 = msg.split(b"\x00", 1)
                tmsgs = tmsg.split(b"\x00")
                msgid1 = msgid1.decode("utf-8")
                for plural_idx, tm in enumerate(tmsgs):
                    self._catalog[(msgid1, plural_idx)] = tm.decode("utf-8")
            else:
                ctx_marker = self.CONTEXT.encode()
                cidx = msg.find(ctx_marker)
                if cidx >= 0:
                    ctx = msg[:cidx].decode("utf-8")
                    msgid = msg[cidx + 1:].decode("utf-8")
                    self._catalog[(ctx, msgid)] = tmsg.decode("utf-8")
                else:
                    self._catalog[msg.decode("utf-8")] = tmsg.decode("utf-8")
            masteridx += 8
            transidx += 8

    def gettext(self, message):
        missing = object()
        tmsg = self._catalog.get(message, missing)
        if tmsg is missing:
            return super().gettext(message)
        return tmsg

    def ngettext(self, singular, plural, n):
        missing = object()
        tmsg = self._catalog.get((singular, self._plural(n)), missing)
        if tmsg is missing:
            return super().ngettext(singular, plural, n)
        return tmsg

    def pgettext(self, context, message):
        missing = object()
        tmsg = self._catalog.get((context, message), missing)
        if tmsg is missing:
            return super().pgettext(context, message)
        return tmsg


def find(domain, localedir=None, languages=None, all=False):
    import os

    if localedir is None:
        localedir = "/usr/share/locale"
    if languages is None:
        languages = []
        for envar in ("LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"):
            val = os.environ.get(envar)
            if val:
                languages = val.split(":")
                break
    languages = list(languages) + ["C"]
    result = []
    for lang in languages:
        if lang == "C":
            break
        mofile = os.path.join(localedir, lang, "LC_MESSAGES", domain + ".mo")
        if os.path.exists(mofile):
            if not all:
                return mofile
            result.append(mofile)
    if all:
        return result
    return None


def translation(domain, localedir=None, languages=None, class_=None, fallback=False):
    if class_ is None:
        class_ = GNUTranslations
    mofiles = find(domain, localedir, languages, all=True)
    if not mofiles:
        if fallback:
            return NullTranslations()
        raise OSError("no translation file found for domain %r" % domain)
    result = None
    for mofile in mofiles:
        fp = open(mofile, "rb")
        t = class_(fp)
        fp.close()
        if result is None:
            result = t
        else:
            result.add_fallback(t)
    return result


_current = NullTranslations()


def install(domain, localedir=None, names=None):
    global _current
    _current = translation(domain, localedir, fallback=True)
    _current.install(names)


def textdomain(domain=None):
    return domain


def bindtextdomain(domain, localedir=None):
    return localedir


def gettext(message):
    return _current.gettext(message)


def ngettext(singular, plural, n):
    return _current.ngettext(singular, plural, n)


def pgettext(context, message):
    return _current.pgettext(context, message)


def npgettext(context, singular, plural, n):
    return _current.npgettext(context, singular, plural, n)


def dgettext(domain, message):
    return gettext(message)


def dngettext(domain, singular, plural, n):
    return ngettext(singular, plural, n)
