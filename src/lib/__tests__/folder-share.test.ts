import { describe, expect, it } from 'vitest';
import { folderShareSummary, isPortableShareName, shareModeLabel } from '../folder-share';

describe('folder share helpers', () => {
  it('accepts the same portable names as the native store', () => {
    for (const name of ['w', 'work', 'build_1', 'src-7ab']) expect(isPortableShareName(name)).toBe(true);
    for (const name of ['', 'toolong8', 'space x', 'dot.ted', 'ümlaut']) expect(isPortableShareName(name)).toBe(false);
  });

  it('summarizes the committed state without implying a grant that is not saved', () => {
    expect(folderShareSummary(null)).toBe('Checking folder sharing…');
    expect(folderShareSummary({ supported: false, grant: null, error: null })).toContain('Windows build');
    expect(folderShareSummary({ supported: true, grant: null, error: null })).toBe('No folder is shared with this host.');
    expect(folderShareSummary({ supported: true, grant: null, error: 'The saved folder shares are invalid, so folder sharing is off.' }))
      .toContain('sharing is off');
    expect(folderShareSummary({
      supported: true, error: null,
      grant: { endpoint: 'h:3389', fingerprint: 'SHA256:x', folder: 'D:\\work', shareName: 'work', mode: 'readWrite' },
    })).toBe('D:\\work as “work”, read and write.');
    expect(shareModeLabel('readOnly')).toBe('Read-only');
  });
});
