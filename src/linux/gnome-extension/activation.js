// Workspace matching for existing VS Code windows, in compositor MRU order.
export function editorWindowIndex(target, windows) {
    if (!target.startsWith('vscode://file/'))
        return -1;
    let path;
    try {
        path = decodeURIComponent(target.slice('vscode://file'.length).split(/[?#]/)[0]);
    } catch {
        return -1;
    }
    const name = path.replace(/\/+$/, '').split('/').pop()?.replace(/\.code-workspace$/, '');
    if (!name)
        return -1;
    const candidates = windows.map((window, index) => ({window, index})).filter(({window}) => {
        const app = (window.appId ?? '').toLowerCase();
        const wmClass = (window.wmClass ?? '').toLowerCase();
        return ['com.microsoft.vscode.desktop', 'com.visualstudio.code.desktop', 'code.desktop'].includes(app)
            || ['code', 'code-oss'].includes(wmClass);
    });
    const fullPath = candidates.filter(({window}) => (window.title ?? '').split(/\s+[—–-]\s+/)
        .some(part => part === path || part.startsWith(`${path} [`)));
    if (fullPath.length === 1)
        return fullPath[0].index;
    const matches = candidates.filter(({window}) => (window.title ?? '').split(/\s+[—–-]\s+/)
        .some(part => part === name || part.startsWith(`${name} [`)));
    // Duplicate workspace names are ambiguous; let VS Code resolve the URI.
    return matches.length === 1 ? matches[0].index : -1;
}
