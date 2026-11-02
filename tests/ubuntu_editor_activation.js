import {editorWindowIndex} from '../src/linux/gnome-extension/activation.js';

const code = title => ({appId: 'com.microsoft.VSCode.desktop', wmClass: 'code', title});
function check(target, windows, expected) {
    const actual = editorWindowIndex(target, windows);
    if (actual !== expected)
        throw new Error(`${target}: expected ${expected}, got ${actual}`);
}
check('vscode://file/home/user/project', [code('readme — other — Visual Studio Code'),
    code('main.rs — project — Visual Studio Code')], 1);
check('vscode://file/home/user/project', [{appId: 'firefox.desktop', title: 'project'},
    code('project-two - Visual Studio Code')], -1);
check('vscode://file/home/user/project', [code('project [SSH: host] - Visual Studio Code')], 0);
check('vscode://file/home/user/My%20Project', [code('My Project — Visual Studio Code')], 0);
check('vscode://file/home/user/a%23b', [code('a#b — Visual Studio Code')], 0);
check('vscode://file/home/user/project.code-workspace', [code('project — Visual Studio Code')], 0);
check('vscode://file/home/user/project', [code('project — Visual Studio Code'),
    code('project — Visual Studio Code')], -1);
check('vscode://file/home/user/project', [code('/home/other/project — Visual Studio Code'),
    code('/home/user/project — Visual Studio Code')], 1);
check('vscode://file/home/user/project', [code('/home/user/project-two — Visual Studio Code')], -1);
check('https://example.com/project', [code('project — Visual Studio Code')], -1);
check('vscode://file/%broken', [code('project — Visual Studio Code')], -1);
check('vscode://file/home/user/project', [], -1);
print('VS Code workspace-window matching checks passed');
