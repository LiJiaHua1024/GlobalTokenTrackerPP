import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../core/theme.dart';

class CurrencySettingsCard extends StatelessWidget {
  const CurrencySettingsCard({super.key});
  @override
  Widget build(BuildContext context) {
    final p = context.watch<ThemeProvider>();
    return Card(
        child: Padding(
            padding: const EdgeInsets.all(20),
            child:
                Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
              Text('显示货币与汇率', style: Theme.of(context).textTheme.titleMedium),
              const SizedBox(height: 12),
              Wrap(
                  spacing: 16,
                  runSpacing: 8,
                  crossAxisAlignment: WrapCrossAlignment.center,
                  children: [
                    DropdownButton<String>(
                        value: p.requestedCurrency,
                        items: [
                          for (final code in {
                            'USD',
                            'CNY',
                            'EUR',
                            'GBP',
                            'JPY',
                            'HKD',
                            'SGD',
                            'AUD',
                            'CAD',
                            'CHF',
                            'KRW', p.requestedCurrency
                          })
                            DropdownMenuItem(value: code, child: Text(code))
                        ],
                        onChanged: p.currencyBusy
                            ? null
                            : (code) async {
                                try {
                                  await p.setCurrency(code!);
                                } catch (e) {
                                  if (context.mounted) {
                                    ScaffoldMessenger.of(context).showSnackBar(
                                        SnackBar(content: Text('切换货币失败：$e')));
                                  }
                                }
                              }),
                    OutlinedButton.icon(
                        onPressed: p.currencyBusy ? null : p.refreshCurrency,
                        icon: const Icon(Icons.currency_exchange),
                        label: Text(p.currencyBusy ? '同步中…' : '立即同步汇率')),
                  ]),
              const SizedBox(height: 8),
              Text(
                  '来源：${p.exchangeSource ?? '暂无缓存'} · 更新：${p.exchangeUpdatedAt == null ? '尚未同步' : DateTime.fromMillisecondsSinceEpoch(p.exchangeUpdatedAt!).toLocal().toString().split('.').first}${p.exchangeStale ? ' · 缓存已过期' : ''}'),
              if (!p.hasExchangeRate) const Text('所选币种尚无有效汇率，金额暂以 USD 显示。'),
              if (p.currencyError != null)
                Text('同步失败，保留原缓存：${p.currencyError}',
                    style:
                        TextStyle(color: Theme.of(context).colorScheme.error)),
              const SizedBox(height: 8),
              const Text('每 12 小时自动同步。汇率只用于显示换算，账本与 CSV 保留美元。'),
            ])));
  }
}
