import 'package:flutter/material.dart';
import 'package:window_manager/window_manager.dart';

class CustomTitleBar extends StatefulWidget {
  const CustomTitleBar({super.key});

  @override
  State<CustomTitleBar> createState() => _CustomTitleBarState();
}

class _CustomTitleBarState extends State<CustomTitleBar> with WindowListener {
  bool _isMaximized = false;

  @override
  void initState() {
    super.initState();
    windowManager.addListener(this);
    _checkMaximized();
  }

  @override
  void dispose() {
    windowManager.removeListener(this);
    super.dispose();
  }

  Future<void> _checkMaximized() async {
    final max = await windowManager.isMaximized();
    if (mounted) {
      setState(() {
        _isMaximized = max;
      });
    }
  }

  @override
  void onWindowMaximize() {
    setState(() => _isMaximized = true);
  }

  @override
  void onWindowUnmaximize() {
    setState(() => _isMaximized = false);
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);

    return Container(
      height: 38,
      decoration: BoxDecoration(
        color: theme.colorScheme.surface,
        border: Border(
          bottom: BorderSide(
            color: theme.colorScheme.outlineVariant.withValues(alpha: 0.3),
            width: 1,
          ),
        ),
      ),
      child: Row(
        children: [
          // Left: Logo & Window Title
          Padding(
            padding: const EdgeInsets.only(left: 12.0, right: 8.0),
            child: Row(
              children: [
                Container(
                  padding: const EdgeInsets.all(4),
                  decoration: BoxDecoration(
                    color: theme.colorScheme.primaryContainer,
                    borderRadius: BorderRadius.circular(6),
                  ),
                  child: Icon(
                    Icons.auto_graph,
                    size: 14,
                    color: theme.colorScheme.primary,
                  ),
                ),
                const SizedBox(width: 8),
                Text(
                  'GlobalTokenTracker++',
                  style: theme.textTheme.labelMedium?.copyWith(
                    fontWeight: FontWeight.bold,
                    color: theme.colorScheme.onSurface,
                    letterSpacing: 0.2,
                  ),
                ),
                const SizedBox(width: 6),
                Container(
                  padding: const EdgeInsets.symmetric(horizontal: 5, vertical: 1),
                  decoration: BoxDecoration(
                    color: theme.colorScheme.surfaceVariant.withValues(alpha: 0.7),
                    borderRadius: BorderRadius.circular(4),
                  ),
                  child: Text(
                    'Material 3',
                    style: TextStyle(
                      fontSize: 9,
                      fontWeight: FontWeight.w600,
                      color: theme.colorScheme.primary,
                    ),
                  ),
                ),
              ],
            ),
          ),

          // Center: Draggable Area (Double click to toggle Maximize)
          Expanded(
            child: GestureDetector(
              behavior: HitTestBehavior.translucent,
              onDoubleTap: () async {
                if (await windowManager.isMaximized()) {
                  await windowManager.unmaximize();
                } else {
                  await windowManager.maximize();
                }
              },
              child: const DragToMoveArea(
                child: SizedBox(
                  height: double.infinity,
                ),
              ),
            ),
          ),

          // Right: Windows Caption Controls (Minimize, Maximize, Close)
          _TitleBarButton(
            icon: Icons.remove,
            tooltip: '最小化',
            onPressed: () => windowManager.minimize(),
          ),
          _TitleBarButton(
            icon: _isMaximized ? Icons.filter_none : Icons.crop_square,
            tooltip: _isMaximized ? '向下还原' : '最大化',
            iconSize: _isMaximized ? 12 : 14,
            onPressed: () async {
              if (await windowManager.isMaximized()) {
                await windowManager.unmaximize();
              } else {
                await windowManager.maximize();
              }
            },
          ),
          _TitleBarButton(
            icon: Icons.close,
            tooltip: '关闭',
            isClose: true,
            onPressed: () => windowManager.close(),
          ),
        ],
      ),
    );
  }
}

class _TitleBarButton extends StatefulWidget {
  final IconData icon;
  final String tooltip;
  final VoidCallback onPressed;
  final bool isClose;
  final double iconSize;

  const _TitleBarButton({
    required this.icon,
    required this.tooltip,
    required this.onPressed,
    this.isClose = false,
    this.iconSize = 14,
  });

  @override
  State<_TitleBarButton> createState() => _TitleBarButtonState();
}

class _TitleBarButtonState extends State<_TitleBarButton> {
  bool _isHovered = false;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);

    Color hoverColor;
    Color iconColor;

    if (widget.isClose) {
      hoverColor = Colors.red.shade600;
      iconColor = _isHovered ? Colors.white : theme.colorScheme.onSurface;
    } else {
      hoverColor = theme.colorScheme.onSurface.withValues(alpha: 0.1);
      iconColor = theme.colorScheme.onSurface;
    }

    return MouseRegion(
      onEnter: (_) => setState(() => _isHovered = true),
      onExit: (_) => setState(() => _isHovered = false),
      child: Tooltip(
        message: widget.tooltip,
        waitDuration: const Duration(milliseconds: 500),
        child: GestureDetector(
          onTap: widget.onPressed,
          child: Container(
            width: 44,
            height: 38,
            color: _isHovered ? hoverColor : Colors.transparent,
            alignment: Alignment.center,
            child: Icon(
              widget.icon,
              size: widget.iconSize,
              color: iconColor,
            ),
          ),
        ),
      ),
    );
  }
}
