import 'package:flutter_test/flutter_test.dart';
import 'package:globaltokentracker_ui/core/models.dart';

void main() {
  test('unknown vendor totals remain separate from input and output', () {
    final raw = {'input_tokens': 10, 'output_tokens': 20, 'unclassified_tokens': 1200};
    final total = Totals.fromJson(raw);
    expect(total.totalTokens, 1230);
    expect(total.inputTokens, 10);
    expect(total.outputTokens, 20);
    expect(AppSummary.fromJson(raw).totalTokens, 1230);
    expect(EventRow.fromJson(raw).unclassifiedTokens, 1200);
    expect(Totals.fromJson({}).unclassifiedTokens, 0);
  });
}
