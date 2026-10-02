import 'dart:io';
import 'package:flutter/material.dart';
import 'package:window_manager/window_manager.dart';
import '../core/app_info.dart';
import '../widgets/custom_title_bar.dart';

class InstallerApp extends StatelessWidget {
  final List<String> args;

  const InstallerApp({super.key, required this.args});

  @override
  Widget build(BuildContext context) {
    final isUninstall = args.contains('--uninstall');
    const seedColor = Color(0xFF1A73E8); // Google Blue M3

    return MaterialApp(
      title: isUninstall ? 'GlobalTokenTracker++ 卸载向导' : 'GlobalTokenTracker++ 安装向导',
      debugShowCheckedModeBanner: false,
      themeMode: ThemeMode.system,
      theme: ThemeData(
        useMaterial3: true,
        brightness: Brightness.light,
        colorScheme: ColorScheme.fromSeed(
          seedColor: seedColor,
          brightness: Brightness.light,
        ),
        fontFamily: 'Segoe UI',
        cardTheme: CardThemeData(
          elevation: 0,
          shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(16)),
          clipBehavior: Clip.antiAlias,
        ),
      ),
      darkTheme: ThemeData(
        useMaterial3: true,
        brightness: Brightness.dark,
        colorScheme: ColorScheme.fromSeed(
          seedColor: seedColor,
          brightness: Brightness.dark,
        ),
        fontFamily: 'Segoe UI',
        cardTheme: CardThemeData(
          elevation: 0,
          shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(16)),
          clipBehavior: Clip.antiAlias,
        ),
      ),
      home: InstallerPage(args: args, isUninstall: isUninstall),
    );
  }
}

class InstallerPage extends StatefulWidget {
  final List<String> args;
  final bool isUninstall;

  const InstallerPage({super.key, required this.args, required this.isUninstall});

  @override
  State<InstallerPage> createState() => _InstallerPageState();
}

enum InstallState { ready, working, done, error }

class _InstallerPageState extends State<InstallerPage> with WindowListener {
  late TextEditingController _pathController;
  bool _shortcutDesktop = true;
  bool _shortcutStartMenu = true;
  bool _addToPath = true;

  InstallState _state = InstallState.ready;
  String _statusMessage = '';
  double _progress = 0.0;
  String? _errorMessage;

  String _installerSource = '';
  String _installerExe = '';
  String _customDest = '';

  /// PowerShell that adds `dest` to (or removes it from) the user PATH.
  ///
  /// The value is read with `DoNotExpandEnvironmentNames` and written back with
  /// the kind it already had, so an entry such as `%USERPROFILE%\...` keeps
  /// expanding instead of being frozen into this profile's literal path. The
  /// environment key is opened read/write as a single handle, and nothing is
  /// written unless the current value could be read: rewriting PATH from an
  /// empty read would drop every entry the installer never saw.
  ///
  /// Entries are compared after `%VAR%` expansion, so an entry stored as
  /// `%LOCALAPPDATA%\...` dedupes against (and uninstalls alongside) its
  /// absolute spelling. Before any overwrite the current value and kind are
  /// backed up to `%USERPROFILE%\.globaltokentracker\path.bak` — a failed
  /// backup aborts the write — and afterwards a `WM_SETTINGCHANGE` broadcast
  /// lets freshly opened terminals see the new PATH without a logoff.
  String _userPathScript({required String dest, required bool remove}) {
    final edit = remove
        ? '''
        \$kept = @(\$parts | Where-Object { (& \$exp \$_) -ine (& \$exp \$d) })
        \$new = \$kept -join ';'
        \$changed = \$kept.Count -ne \$parts.Count
        '''
        : '''
        \$exists = \$false
        foreach (\$p in \$parts) { if ((& \$exp \$p) -ieq (& \$exp \$d)) { \$exists = \$true; break } }
        \$new = if (-not \$exists) { \$(if (\$cur.Length -gt 0) { "\$cur;$dest" } else { "$dest" }) } else { \$cur }
        \$changed = -not \$exists
        ''';
    return '''
        \$name = 'Path'
        \$d = "$dest".TrimEnd('\\')
        \$key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', \$true)
        if (-not \$key) { throw 'cannot open HKCU environment key' }
        try {
            \$has = \$key.GetValueNames() -contains \$name
            \$kind = if (\$has) { \$key.GetValueKind(\$name) } else { 'ExpandString' }
            if (\$has -and \$kind -ne 'String' -and \$kind -ne 'ExpandString') { throw "unexpected \$name kind: \$kind" }
            \$cur = if (\$has) { [string]\$key.GetValue(\$name, '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames) } else { '' }
            \$exp = { param(\$p) [Environment]::ExpandEnvironmentVariables(\$p.Trim().TrimEnd('\\')) }
            \$parts = @(\$cur -split ';')
$edit
            if (\$changed) {
                if (\$has) {
                    \$bakDir = Join-Path \$env:USERPROFILE '.globaltokentracker'
                    New-Item -ItemType Directory -Force -Path \$bakDir | Out-Null
                    Set-Content -Path (Join-Path \$bakDir 'path.bak') -Value @("type=\$kind", \$cur) -Encoding UTF8 -ErrorAction Stop
                }
                \$key.SetValue(\$name, \$new, \$kind)
                try {
                    Add-Type -Namespace Win32 -Name NativeEnv -MemberDefinition '[DllImport("user32.dll", SetLastError = true, CharSet = CharSet.Unicode)] public static extern IntPtr SendMessageTimeout(System.IntPtr hWnd, uint Msg, System.UIntPtr wParam, string lParam, uint fuFlags, uint uTimeout, out System.UIntPtr lpdwResult);' -ErrorAction Stop
                    \$res = [System.UIntPtr]::Zero
                    [Win32.NativeEnv]::SendMessageTimeout([System.IntPtr]0xFFFF, 0x1A, [System.UIntPtr]::Zero, 'Environment', 2, 5000, [ref]\$res) | Out-Null
                } catch { }
            }
        } finally {
            \$key.Close()
        }
        ''';
  }

  @override
  void initState() {
    super.initState();

    // Parse CLI parameters
    for (int i = 0; i < widget.args.length; i++) {
      if (widget.args[i] == '--installer-source' && i + 1 < widget.args.length) {
        _installerSource = widget.args[i + 1];
      }
      if (widget.args[i] == '--installer-exe' && i + 1 < widget.args.length) {
        _installerExe = widget.args[i + 1];
      }
      if (widget.args[i] == '--dest' && i + 1 < widget.args.length) {
        _customDest = widget.args[i + 1];
      }
    }

    // Default install location: %LOCALAPPDATA%\Programs\GlobalTokenTrackerPP
    final localAppData = Platform.environment['LOCALAPPDATA'] ?? r'C:\Users\Default\AppData\Local';
    final defaultDest = _customDest.isNotEmpty ? _customDest : '$localAppData\\Programs\\GlobalTokenTrackerPP';

    _pathController = TextEditingController(text: defaultDest);
    windowManager.addListener(this);
  }

  @override
  void dispose() {
    windowManager.removeListener(this);
    _pathController.dispose();
    super.dispose();
  }

  @override
  void onWindowClose() async {
    // Only reached while preventClose is armed (a step is running): the
    // close click must not kill a half-applied install.
    if (!mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(content: Text('正在安装/卸载，请等待完成后再关闭窗口')),
    );
  }

  /// setState that no-ops once the widget is gone — the install/uninstall
  /// steps await external processes and must not touch state afterwards.
  void _safeSetState(VoidCallback fn) {
    if (mounted) setState(fn);
  }

  Future<void> _browseFolder() async {
    try {
      final initial = _pathController.text.trim();
      final res = await Process.run('powershell', [
        '-NoProfile',
        '-NonInteractive',
        '-Command',
        '''
        Add-Type -AssemblyName System.Windows.Forms
        \$d = New-Object System.Windows.Forms.FolderBrowserDialog
        \$d.Description = '选择 GlobalTokenTracker++ 安装目录'
        \$d.SelectedPath = '$initial'
        if (\$d.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {
            Write-Output \$d.SelectedPath
        }
        ''',
      ]);
      if (res.exitCode == 0 && res.stdout.toString().trim().isNotEmpty) {
        _safeSetState(() {
          _pathController.text = res.stdout.toString().trim();
        });
      }
    } catch (e) {
      debugPrint("Browse error: $e");
    }
  }

  Future<void> _startInstall() async {
    final dest = _pathController.text.trim();
    if (dest.isEmpty) {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text('安装目录不能为空')),
      );
      return;
    }

    _safeSetState(() {
      _state = InstallState.working;
      _statusMessage = '正在准备安装环境...';
      _progress = 0.05;
    });
    // Block the window close button while a step is running.
    await windowManager.setPreventClose(true);

    try {
      // 1. Terminate old running process (excluding current installer process)
      _safeSetState(() {
        _statusMessage = '结束正在运行的实例...';
        _progress = 0.10;
      });
      final myPid = pid;
      await Process.run('taskkill', [
        '/F',
        '/FI',
        'PID ne $myPid',
        '/IM',
        'globaltokentracker_ui.exe',
      ]);

      // 2. Create destination directory
      final destDir = Directory(dest);
      if (!await destDir.exists()) {
        await destDir.create(recursive: true);
      }

      // 3. Copy files from installer source
      final srcDir = _installerSource.isNotEmpty ? Directory(_installerSource) : File(Platform.resolvedExecutable).parent;
      final entities = await srcDir.list(recursive: true).toList();
      final totalFiles = entities.whereType<File>().length;

      int copiedFiles = 0;
      for (final entity in entities) {
        final relativePath = entity.path.substring(srcDir.path.length).replaceFirst(RegExp(r'^[/\\]'), '');
        
        // Skip installer helper launcher
        if (relativePath.toLowerCase() == 'installer_ui.exe') {
          continue;
        }

        final targetPath = '$dest\\$relativePath';

        if (entity is Directory) {
          final d = Directory(targetPath);
          if (!await d.exists()) await d.create(recursive: true);
        } else if (entity is File) {
          final targetFile = File(targetPath);
          final parent = targetFile.parent;
          if (!await parent.exists()) await parent.create(recursive: true);

          await entity.copy(targetPath);
          copiedFiles++;

          if (mounted) {
            _safeSetState(() {
              _statusMessage = '正在释放: $relativePath';
              _progress = 0.15 + (0.55 * (copiedFiles / (totalFiles > 0 ? totalFiles : 1)));
            });
          }
        }
      }

      // 4. Save installer as uninstaller
      if (_installerExe.isNotEmpty && await File(_installerExe).exists()) {
        final targetUninstaller = '$dest\\globaltokentrackerpp-setup.exe';
        if (_installerExe.toLowerCase() != targetUninstaller.toLowerCase()) {
          await File(_installerExe).copy(targetUninstaller);
        }
      }

      // 5. Register in Windows Uninstall
      _safeSetState(() {
        _statusMessage = '写入 Windows 卸载注册信息...';
        _progress = 0.75;
      });
      final regCmd = '''
      \$reg = "HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\GlobalTokenTrackerPP"
      New-Item -Path \$reg -Force | Out-Null
      Set-ItemProperty -Path \$reg -Name "DisplayName" -Value "GlobalTokenTracker++"
      Set-ItemProperty -Path \$reg -Name "DisplayVersion" -Value "${AppInfo.currentVersion}"
      Set-ItemProperty -Path \$reg -Name "Publisher" -Value "GlobalTokenTracker++"
      Set-ItemProperty -Path \$reg -Name "InstallLocation" -Value "$dest"
      Set-ItemProperty -Path \$reg -Name "DisplayIcon" -Value "$dest\\globaltokentracker_ui.exe"
      Set-ItemProperty -Path \$reg -Name "UninstallString" -Value "`"$dest\\globaltokentrackerpp-setup.exe`" --uninstall"
      Set-ItemProperty -Path \$reg -Name "QuietUninstallString" -Value "`"$dest\\globaltokentrackerpp-setup.exe`" --uninstall --quiet"
      Set-ItemProperty -Path \$reg -Name "NoModify" -Value 1 -Type DWord
      Set-ItemProperty -Path \$reg -Name "NoRepair" -Value 1 -Type DWord
      ''';
      await Process.run('powershell', ['-NoProfile', '-NonInteractive', '-Command', regCmd]);

      // 6. Create Shortcuts
      if (_shortcutStartMenu || _shortcutDesktop) {
        _safeSetState(() {
          _statusMessage = '创建快捷方式...';
          _progress = 0.85;
        });

        final appData = Platform.environment['APPDATA'] ?? '';
        final startMenuDir = '$appData\\Microsoft\\Windows\\Start Menu\\Programs\\GlobalTokenTracker++';

        final shortcutCmd = '''
        \$ws = New-Object -ComObject WScript.Shell
        if ("$_shortcutStartMenu" -eq "True") {
            New-Item -ItemType Directory -Force -Path "$startMenuDir" | Out-Null
            \$s = \$ws.CreateShortcut("$startMenuDir\\GlobalTokenTracker++.lnk")
            \$s.TargetPath = "$dest\\globaltokentracker_ui.exe"
            \$s.WorkingDirectory = "$dest"
            \$s.Save()

            \$u = \$ws.CreateShortcut("$startMenuDir\\卸载 GlobalTokenTracker++.lnk")
            \$u.TargetPath = "$dest\\globaltokentrackerpp-setup.exe"
            \$u.Arguments = "--uninstall"
            \$u.WorkingDirectory = "$dest"
            \$u.Save()
        }
        if ("$_shortcutDesktop" -eq "True") {
            \$desktopPath = [System.Environment]::GetFolderPath('Desktop')
            \$d = \$ws.CreateShortcut("\$desktopPath\\GlobalTokenTracker++.lnk")
            \$d.TargetPath = "$dest\\globaltokentracker_ui.exe"
            \$d.WorkingDirectory = "$dest"
            \$d.Save()
        }
        ''';
        await Process.run('powershell', ['-NoProfile', '-NonInteractive', '-Command', shortcutCmd]);
      }

      // 7. Add to User PATH
      if (_addToPath) {
        _safeSetState(() {
          _statusMessage = '追加用户环境变量 PATH...';
          _progress = 0.95;
        });
        final pathRes = await Process.run('powershell', [
          '-NoProfile',
          '-NonInteractive',
          '-Command',
          _userPathScript(dest: dest, remove: false),
        ]);
        if (pathRes.exitCode != 0) {
          debugPrint('用户 PATH 未修改：${pathRes.stderr}');
        }
      }

      _safeSetState(() {
        _state = InstallState.done;
        _statusMessage = '安装完成';
        _progress = 1.0;
      });
    } catch (e, st) {
      debugPrint("Install error: $e\n$st");
      _safeSetState(() {
        _state = InstallState.error;
        _errorMessage = e.toString();
      });
    } finally {
      await windowManager.setPreventClose(false);
    }
  }

  /// Strip trailing backslashes; a bare drive root (`D:\`) is left intact so
  /// [_looksLikeInstallDir] can refuse it instead of trimming it to `D:`.
  static String _normalizedDest(String raw) {
    var d = raw.trim();
    while (d.length > 3 && d.endsWith('\\')) {
      d = d.substring(0, d.length - 1);
    }
    return d;
  }

  /// Only a folder that actually contains our binaries may be uninstalled —
  /// belt-and-braces alongside the identical guard in the Rust setup exe.
  static bool _looksLikeInstallDir(String dest) {
    if (RegExp(r'^[A-Za-z]:\\?$').hasMatch(dest)) return false; // drive root
    final exe = File('$dest\\globaltokentracker_ui.exe');
    final setup = File('$dest\\globaltokentrackerpp-setup.exe');
    return exe.existsSync() || setup.existsSync();
  }

  Future<void> _startUninstall() async {
    _safeSetState(() {
      _state = InstallState.working;
      _statusMessage = '正在结束运行中的进程...';
      _progress = 0.2;
    });
    // Block the window close button while a step is running.
    await windowManager.setPreventClose(true);

    try {
      // 0. Guard: never run destructive steps against a directory that does
      // not contain our own binaries (or a drive root) — the wizard may have
      // been launched from an arbitrary folder such as Downloads.
      final dest = _normalizedDest(_pathController.text.trim());
      if (!_looksLikeInstallDir(dest)) {
        _safeSetState(() {
          _state = InstallState.error;
          _errorMessage =
              '该目录不是本应用的安装目录（缺少主程序），已中止卸载。\n目标目录：$dest';
        });
        return;
      }

      // 1. Kill running instances (excluding current process)
      final myPid = pid;
      await Process.run('taskkill', [
        '/F',
        '/FI',
        'PID ne $myPid',
        '/IM',
        'globaltokentracker_ui.exe',
      ]);

      // 2. Remove shortcuts
      _safeSetState(() {
        _statusMessage = '移除快捷方式...';
        _progress = 0.4;
      });
      final appData = Platform.environment['APPDATA'] ?? '';
      final startMenuDir = Directory('$appData\\Microsoft\\Windows\\Start Menu\\Programs\\GlobalTokenTracker++');
      if (await startMenuDir.exists()) {
        await startMenuDir.delete(recursive: true);
      }

      final removeShortcutsCmd = '''
      \$desktopPath = [System.Environment]::GetFolderPath('Desktop')
      Remove-Item "\$desktopPath\\GlobalTokenTracker++.lnk" -Force -ErrorAction SilentlyContinue
      Remove-Item "\$env:USERPROFILE\\Desktop\\GlobalTokenTracker++.lnk" -Force -ErrorAction SilentlyContinue
      ''';
      await Process.run('powershell', ['-NoProfile', '-NonInteractive', '-Command', removeShortcutsCmd]);

      // 3. Remove Registry Key & PATH
      _safeSetState(() {
        _statusMessage = '清理系统卸载项与环境变量...';
        _progress = 0.6;
      });
      final cleanRegCmd = '''
      Remove-Item -Path "HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\GlobalTokenTrackerPP" -Recurse -Force -ErrorAction SilentlyContinue
      ''';
      await Process.run('powershell', ['-NoProfile', '-NonInteractive', '-Command', cleanRegCmd]);

      final pathRes = await Process.run('powershell', [
        '-NoProfile',
        '-NonInteractive',
        '-Command',
        _userPathScript(dest: dest, remove: true),
      ]);
      if (pathRes.exitCode != 0) {
        debugPrint('用户 PATH 未清理：${pathRes.stderr}');
      }

      // 4. Remove install folder
      _safeSetState(() {
        _statusMessage = '删除程序文件...';
        _progress = 0.8;
      });
      final installDir = Directory(dest);
      if (await installDir.exists()) {
        // Schedule deferred deletion via cmd
        Process.start('cmd', ['/C', 'ping 127.0.0.1 -n 2 >nul & rmdir /S /Q "$dest"'], mode: ProcessStartMode.detached);
      }

      _safeSetState(() {
        _state = InstallState.done;
        _statusMessage = '卸载完成';
        _progress = 1.0;
      });
    } catch (e, st) {
      debugPrint("Uninstall error: $e\n$st");
      _safeSetState(() {
        _state = InstallState.error;
        _errorMessage = e.toString();
      });
    } finally {
      await windowManager.setPreventClose(false);
    }
  }

  Future<void> _launchInstalledApp() async {
    final dest = _pathController.text.trim();
    final exe = '$dest\\globaltokentracker_ui.exe';
    if (await File(exe).exists()) {
      // The installer's cwd is a temp extract dir that gets scheduled for
      // deletion right after this — the new app must not inherit it.
      await Process.start(exe, [],
          workingDirectory: dest, mode: ProcessStartMode.detached);
    }
    await windowManager.close();
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);

    return Scaffold(
      backgroundColor: theme.colorScheme.surface,
      body: Column(
        children: [
          CustomTitleBar(
            title: widget.isUninstall ? 'GlobalTokenTracker++ 卸载向导' : 'GlobalTokenTracker++ 安装向导',
            canMaximize: false,
          ),

          Expanded(
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 28.0, vertical: 16.0),
              child: AnimatedSwitcher(
                duration: const Duration(milliseconds: 300),
                child: _buildBody(theme),
              ),
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildBody(ThemeData theme) {
    if (widget.isUninstall) {
      return _buildUninstallView(theme);
    }

    switch (_state) {
      case InstallState.ready:
        return _buildReadyView(theme);
      case InstallState.working:
        return _buildWorkingView(theme);
      case InstallState.done:
        return _buildDoneView(theme);
      case InstallState.error:
        return _buildErrorView(theme);
    }
  }

  Widget _buildReadyView(ThemeData theme) {
    return Column(
      key: const ValueKey('ready_view'),
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        // App Header Brand Card
        Row(
          children: [
            Container(
              width: 56,
              height: 56,
              decoration: BoxDecoration(
                borderRadius: BorderRadius.circular(16),
                color: theme.colorScheme.primaryContainer.withValues(alpha: 0.7),
                boxShadow: [
                  BoxShadow(
                    color: theme.colorScheme.primary.withValues(alpha: 0.25),
                    blurRadius: 16,
                    offset: const Offset(0, 4),
                  ),
                ],
              ),
              child: ClipRRect(
                borderRadius: BorderRadius.circular(16),
                child: Image.asset('assets/icon-64.png', errorBuilder: (_, __, ___) => const Icon(Icons.token, size: 36)),
              ),
            ),
            const SizedBox(width: 16),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    'GlobalTokenTracker++',
                    style: theme.textTheme.titleLarge?.copyWith(
                      fontWeight: FontWeight.bold,
                      letterSpacing: -0.3,
                    ),
                  ),
                  const SizedBox(height: 4),
                  Text(
                    'Material Design 3 本地 AI 编码工具用量分析中心',
                    style: theme.textTheme.bodyMedium?.copyWith(
                      color: theme.colorScheme.outline,
                    ),
                  ),
                ],
              ),
            ),
          ],
        ),
        const SizedBox(height: 20),

        // Path Selection Card
        Card(
          elevation: 0,
          color: theme.colorScheme.surfaceContainerHighest.withValues(alpha: 0.4),
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(16),
            side: BorderSide(color: theme.colorScheme.outlineVariant.withValues(alpha: 0.3)),
          ),
          child: Padding(
            padding: const EdgeInsets.all(16.0),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  '安装路径选择',
                  style: theme.textTheme.titleSmall?.copyWith(fontWeight: FontWeight.bold),
                ),
                const SizedBox(height: 10),
                Row(
                  children: [
                    Expanded(
                      child: TextField(
                        controller: _pathController,
                        style: const TextStyle(fontSize: 13, fontFamily: 'Segoe UI'),
                        decoration: InputDecoration(
                          prefixIcon: const Icon(Icons.folder_outlined, size: 20),
                          isDense: true,
                          contentPadding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
                          border: OutlineInputBorder(borderRadius: BorderRadius.circular(10)),
                        ),
                      ),
                    ),
                    const SizedBox(width: 10),
                    OutlinedButton.icon(
                      onPressed: _browseFolder,
                      icon: const Icon(Icons.folder_open, size: 18),
                      label: const Text('浏览...'),
                    ),
                  ],
                ),
                const SizedBox(height: 8),
                Text(
                  '与原版 GlobalTokenTracker 完全独立隔离，不会产生任何覆盖或冲突。',
                  style: TextStyle(fontSize: 11, color: theme.colorScheme.outline),
                ),
              ],
            ),
          ),
        ),
        const SizedBox(height: 12),

        // Options Card
        Card(
          elevation: 0,
          color: theme.colorScheme.surfaceContainerHighest.withValues(alpha: 0.4),
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(16),
            side: BorderSide(color: theme.colorScheme.outlineVariant.withValues(alpha: 0.3)),
          ),
          child: Column(
            children: [
              CheckboxListTile(
                value: _shortcutDesktop,
                onChanged: (v) => setState(() => _shortcutDesktop = v ?? true),
                title: const Text('创建桌面快捷方式', style: TextStyle(fontSize: 13)),
                dense: true,
                visualDensity: VisualDensity.compact,
              ),
              CheckboxListTile(
                value: _shortcutStartMenu,
                onChanged: (v) => setState(() => _shortcutStartMenu = v ?? true),
                title: const Text('创建开始菜单快捷方式', style: TextStyle(fontSize: 13)),
                dense: true,
                visualDensity: VisualDensity.compact,
              ),
              CheckboxListTile(
                value: _addToPath,
                onChanged: (v) => setState(() => _addToPath = v ?? true),
                title: const Text('添加到用户环境变量 PATH', style: TextStyle(fontSize: 13)),
                dense: true,
                visualDensity: VisualDensity.compact,
              ),
            ],
          ),
        ),

        const Spacer(),

        // Bottom Actions
        Row(
          mainAxisAlignment: MainAxisAlignment.end,
          children: [
            TextButton(
              onPressed: () => windowManager.close(),
              child: const Text('取消'),
            ),
            const SizedBox(width: 12),
            FilledButton.icon(
              onPressed: _startInstall,
              icon: const Icon(Icons.download_done, size: 18),
              label: const Text('立即安装'),
              style: FilledButton.styleFrom(
                padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 12),
                shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(12)),
              ),
            ),
          ],
        ),
      ],
    );
  }

  Widget _buildWorkingView(ThemeData theme) {
    return Center(
      key: const ValueKey('working_view'),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          const CircularProgressIndicator(strokeWidth: 3),
          const SizedBox(height: 24),
          Text(
            widget.isUninstall ? '正在卸载 GlobalTokenTracker++' : '正在安装 GlobalTokenTracker++',
            style: theme.textTheme.titleMedium?.copyWith(fontWeight: FontWeight.bold),
          ),
          const SizedBox(height: 12),
          Text(
            _statusMessage,
            style: TextStyle(color: theme.colorScheme.outline, fontSize: 13),
            textAlign: TextAlign.center,
          ),
          const SizedBox(height: 24),
          SizedBox(
            width: 360,
            child: ClipRRect(
              borderRadius: BorderRadius.circular(4),
              child: LinearProgressIndicator(
                value: _progress > 0 ? _progress : null,
                minHeight: 6,
              ),
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildDoneView(ThemeData theme) {
    return Center(
      key: const ValueKey('done_view'),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Container(
            padding: const EdgeInsets.all(16),
            decoration: BoxDecoration(
              shape: BoxShape.circle,
              color: Colors.green.withValues(alpha: 0.15),
            ),
            child: const Icon(Icons.check_circle, size: 64, color: Colors.green),
          ),
          const SizedBox(height: 16),
          Text(
            widget.isUninstall ? '卸载完成' : '安装成功！',
            style: theme.textTheme.headlineSmall?.copyWith(fontWeight: FontWeight.bold),
          ),
          const SizedBox(height: 8),
          Text(
            widget.isUninstall
                ? '所有程序文件与快捷方式已完全移除。\n用户 SQLite 账本文件 (%USERPROFILE%\\.globaltokentracker) 依然妥善保留。'
                : 'GlobalTokenTracker++ 已成功就绪，享受极致流畅的 Material Design 3 体验。',
            textAlign: TextAlign.center,
            style: TextStyle(color: theme.colorScheme.outline, fontSize: 13),
          ),
          const SizedBox(height: 32),
          Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              OutlinedButton(
                onPressed: () => windowManager.close(),
                child: const Text('退出向导'),
              ),
              if (!widget.isUninstall) ...[
                const SizedBox(width: 16),
                FilledButton.icon(
                  onPressed: _launchInstalledApp,
                  icon: const Icon(Icons.rocket_launch, size: 18),
                  label: const Text('立即启动'),
                  style: FilledButton.styleFrom(
                    padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 12),
                    shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(12)),
                  ),
                ),
              ],
            ],
          ),
        ],
      ),
    );
  }

  Widget _buildErrorView(ThemeData theme) {
    return Center(
      key: const ValueKey('error_view'),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          const Icon(Icons.error_outline, size: 64, color: Colors.red),
          const SizedBox(height: 16),
          Text('操作失败', style: theme.textTheme.titleLarge?.copyWith(fontWeight: FontWeight.bold)),
          const SizedBox(height: 8),
          Text(_errorMessage ?? '未知错误', style: const TextStyle(color: Colors.red, fontSize: 13)),
          const SizedBox(height: 24),
          OutlinedButton(
            onPressed: () => windowManager.close(),
            child: const Text('关闭'),
          ),
        ],
      ),
    );
  }

  Widget _buildUninstallView(ThemeData theme) {
    if (_state != InstallState.ready) {
      return _buildBody(theme);
    }

    final localAppData = Platform.environment['LOCALAPPDATA'] ?? r'C:\Users\Default\AppData\Local';
    final defaultDest = '$localAppData\\Programs\\GlobalTokenTrackerPP';
    // Show the directory that will actually be deleted, not a hardcoded
    // default — a custom-dir install must be confirmed against its real path.
    final chosen = _normalizedDest(_pathController.text.trim());
    final shownDest = chosen.isNotEmpty ? chosen : defaultDest;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Row(
          children: [
            Container(
              padding: const EdgeInsets.all(12),
              decoration: BoxDecoration(
                color: theme.colorScheme.errorContainer.withValues(alpha: 0.5),
                borderRadius: BorderRadius.circular(12),
              ),
              child: Icon(Icons.delete_outline, size: 32, color: theme.colorScheme.error),
            ),
            const SizedBox(width: 16),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text('确认卸载 GlobalTokenTracker++？', style: theme.textTheme.titleMedium?.copyWith(fontWeight: FontWeight.bold)),
                  const SizedBox(height: 4),
                  Text('卸载将清理程序安装目录及快捷方式', style: TextStyle(color: theme.colorScheme.outline, fontSize: 13)),
                ],
              ),
            ),
          ],
        ),
        const SizedBox(height: 20),
        Card(
          elevation: 0,
          color: theme.colorScheme.surfaceContainerHighest.withValues(alpha: 0.3),
          shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(16)),
          child: Padding(
            padding: const EdgeInsets.all(16.0),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  '将被移除的内容：',
                  style: theme.textTheme.bodyMedium?.copyWith(fontWeight: FontWeight.bold),
                ),
                const SizedBox(height: 8),
                Text(
                  '• 目标目录：$shownDest',
                  style: TextStyle(fontSize: 12, color: theme.colorScheme.onSurfaceVariant),
                ),
                Text(
                  '• 桌面及开始菜单的全部快捷方式',
                  style: TextStyle(fontSize: 12, color: theme.colorScheme.onSurfaceVariant),
                ),
                Text(
                  '• Windows 注册表卸载条目',
                  style: TextStyle(fontSize: 12, color: theme.colorScheme.onSurfaceVariant),
                ),
                const SizedBox(height: 12),
                Container(
                  padding: const EdgeInsets.all(10),
                  decoration: BoxDecoration(
                    color: theme.colorScheme.primaryContainer.withValues(alpha: 0.3),
                    borderRadius: BorderRadius.circular(8),
                  ),
                  child: Row(
                    children: [
                      Icon(Icons.shield_outlined, size: 18, color: theme.colorScheme.primary),
                      const SizedBox(width: 8),
                      Expanded(
                        child: Text(
                          '用户 SQLite 账本文件 (%USERPROFILE%\\.globaltokentracker) 将妥善保留，防止历史数据丢失。',
                          style: TextStyle(fontSize: 11, color: theme.colorScheme.primary),
                        ),
                      ),
                    ],
                  ),
                ),
              ],
            ),
          ),
        ),
        const Spacer(),
        Row(
          mainAxisAlignment: MainAxisAlignment.end,
          children: [
            TextButton(
              onPressed: () => windowManager.close(),
              child: const Text('取消'),
            ),
            const SizedBox(width: 12),
            FilledButton(
              onPressed: _startUninstall,
              style: FilledButton.styleFrom(
                backgroundColor: theme.colorScheme.error,
                foregroundColor: theme.colorScheme.onError,
              ),
              child: const Text('确认卸载'),
            ),
          ],
        ),
      ],
    );
  }
}
