package history

import "os"

func (s *Store) openForTest(note string) (*os.File, error) {
	return os.OpenFile(s.file(note), os.O_APPEND|os.O_WRONLY, 0o600)
}
