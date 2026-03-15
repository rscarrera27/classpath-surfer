## Summary / 요약

<!-- Brief description of the changes / 변경 사항에 대한 간단한 설명 -->

## Checklist / 체크리스트

- [ ] `cargo fmt -- --check` passes / 통과
- [ ] `cargo clippy -- -D warnings` passes / 통과
- [ ] `cargo test` passes / 통과
- [ ] If index schema changed: reader/writer are synchronized / 인덱스 스키마 변경 시: reader/writer 동기화 완료
- [ ] If new subcommand added: `Commands` enum updated, `main.rs` match arm added / 새 서브커맨드 추가 시: `Commands` enum 업데이트, `main.rs` match arm 추가
