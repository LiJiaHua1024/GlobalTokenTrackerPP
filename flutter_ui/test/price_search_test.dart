import 'package:flutter_test/flutter_test.dart';
import 'package:globaltokentracker_ui/core/models.dart';
void main() {
  test('price search ignores separators and supports aliases and dispute filters', () {
    final rows = [
      PriceRow.fromJson({'model':'claude-opus-4-6','agree':2,'total':3,'quotes':[{'source':'a','key':'anthropic/claude-opus-4.6','stance':'Dissents'}]}),
      PriceRow.fromJson({'model':'gpt-5-mini','agree':3,'total':3}),
    ];
    expect(filterPriceRows(rows, 'OPUS 4.6').single.model, 'claude-opus-4-6');
    expect(filterPriceRows(rows, 'gpt mini').single.model, 'gpt-5-mini');
    expect(filterPriceRows(rows, '', disputedOnly:true).single.agree, 2);
    expect(filterPriceRows(rows, 'gpt', disputedOnly:true), isEmpty);
    expect(rows.first.quotes.single.stance, 'dissents');
  });
}
