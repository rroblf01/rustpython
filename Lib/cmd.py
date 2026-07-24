"""A generic class to build line-oriented command interpreters."""


class Cmd:
    """Simple command interpreter implementation."""

    prompt = '(Cmd) '
    identchars = 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_'
    ruler = '='
    lastcmd = ''
    intro = None
    doc_leader = ""
    doc_header = "Documented commands (type help <topic>):"
    misc_header = "Miscellaneous help topics:"
    undoc_header = "Undocumented commands:"
    nohelp = "*** No help on %s"
    use_rawinput = 1

    def __init__(self, completekey='tab', stdin=None, stdout=None):
        if stdin is not None:
            self.stdin = stdin
        else:
            self.stdin = __import__('sys').stdin
        if stdout is not None:
            self.stdout = stdout
        else:
            self.stdout = __import__('sys').stdout
        self.cmdqueue = []
        self.curqueue = []

    def cmdloop(self, intro=None):
        self.preloop()
        if self.use_rawinput and self.completekey:
            try:
                import readline
                self.old_completer = readline.get_completer()
                readline.set_completer(self.complete)
                readline.parse_and_bind(self.completekey + ": complete")
            except ImportError:
                pass
        try:
            if intro is not None:
                self.intro = intro
            if self.intro:
                print(self.intro, file=self.stdout)
            self._cmdloop()
        finally:
            self.postloop()

    def _cmdloop(self):
        stop = None
        while not stop:
            if self.cmdqueue:
                line = self.cmdqueue.pop(0)
            else:
                try:
                    if self.use_rawinput:
                        line = input(self.prompt)
                    else:
                        self.stdout.write(self.prompt)
                        self.stdout.flush()
                        line = self.stdin.readline()
                        if not len(line):
                            line = 'EOF'
                        else:
                            line = line.rstrip('\r\n')
                except EOFError:
                    line = 'EOF'
                except KeyboardInterrupt:
                    self.stdout.write('\n')
                    continue
            if line == 'EOF':
                self.stdout.write('\n')
                break
            if line.rstrip():
                self.lastcmd = line
            stop = self.onecmd(line)
        return stop

    def onecmd(self, line):
        line = line.strip()
        if not line:
            return self.emptyline()
        i, n = 0, len(line)
        while i < n and line[i] in self.identchars:
            i += 1
        cmd = line[:i]
        arg = line[i:].strip()
        stop = None
        if cmd == '':
            stop = self.default(line)
        else:
            try:
                func = getattr(self, 'do_' + cmd)
            except AttributeError:
                stop = self.default(line)
            else:
                stop = func(arg)
        return stop

    def default(self, line):
        print(f"*** Unknown syntax: {line}", file=self.stdout)

    def emptyline(self):
        if self.lastcmd:
            return self.onecmd(self.lastcmd)
        return None

    def do_help(self, arg):
        if arg:
            try:
                func = getattr(self, 'help_' + arg)
            except AttributeError:
                try:
                    doc = getattr(self, 'do_' + arg).__doc__
                    if doc:
                        self.stdout.write(str(doc) + '\n')
                        return
                except AttributeError:
                    pass
                self.stdout.write(self.nohelp % arg + '\n')
                return
            func()
        else:
            names = sorted(dir(self))
            cmds = [n[3:] for n in names if n.startswith('do_')]
            self.stdout.write(str(self.doc_leader) + '\n')
            self.print_topics(self.doc_header, cmds, 15, 80)
            self.print_topics(self.misc_header, [], 15, 80)
            self.print_topics(self.undoc_header, [], 15, 80)

    def print_topics(self, header, cmds, cmdlen, maxcol):
        if not cmds:
            return
        self.stdout.write(str(header) + '\n')
        if self.ruler:
            self.stdout.write(str(self.ruler) * len(header) + '\n')
        self.columnize(cmds, maxcol-1)
        self.stdout.write('\n')

    def columnize(self, list, displaywidth=80):
        if not list:
            self.stdout.write('\n')
            return
        nonempty = [str(x) for x in list]
        width = max(len(x) for x in nonempty) + 1
        ncols = max(1, int((displaywidth + 1) / (width + 1)))
        if ncols > len(nonempty):
            ncols = len(nonempty)
        for i in range(0, len(nonempty), ncols):
            row = nonempty[i:i+ncols]
            self.stdout.write('  '.join(row) + '\n')

    def complete(self, text, state):
        return None

    def preloop(self):
        pass

    def postloop(self):
        pass
