// Links in Note widgets: http(s) URLs open in a new tab, paths of this app navigate in it, nothing else is a link.
import assert from 'node:assert/strict';
import test from 'node:test';
import { linkKind } from '../web/src/pages/dashboards/widgets/markdownBlocks.ts';

test('notes link to the web and to pages of the app, and nowhere else', () => {
  assert.equal(linkKind('https://docs.example.com/aws'), 'external');
  assert.equal(linkKind('http://intake.observecraft.example.com'), 'external');
  assert.equal(linkKind('/integrations/aws'), 'internal');
  assert.equal(linkKind('/dashboards/aws-overview?tpl_var_region=eu-west-1'), 'internal');
  for (const href of ['//evil.example.com', '/\\evil.example.com', 'javascript:alert(1)', 'data:text/html,x', 'integrations/aws', 'mailto:ops@example.com']) assert.equal(linkKind(href), null, href);
});

test('headings go down to ######, as integrations-core notes write them', async () => {
  const { markdownBlocks } = await import('../web/src/pages/dashboards/widgets/markdownBlocks.ts');
  assert.deepEqual(markdownBlocks('#### Further reading\n###### Small'), [
    { type: 'heading', level: 4, text: 'Further reading' },
    { type: 'heading', level: 6, text: 'Small' },
  ]);
  assert.deepEqual(markdownBlocks('####### not a heading'), [{ type: 'paragraph', text: '####### not a heading' }]);
});
