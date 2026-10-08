# AEGIS: реальное развёртывание backend и проверка двух ИИ

## Что действительно запущено
Использованы исходники PR #55, отдельный каталог /home/user/aegis-live-model и порт 11435. Выполнен настоящий Runtime::install через cargo example local_ai, то есть backend установки из приложения. Скачан официальный Ollama v0.35.1 (архив 1 439 658 961 байт, проверен checksum) и Qwen3:8b (основной blob 5 225 374 496 байт). Установка завершилась: installed/server_ready/model_downloaded/model_loaded/owned_server=true, version=0.35.1, size_vram=0. Модель реально работала на CPU. После тестов управляемые процессы остановлены.

## Важная граница
Нативная кнопка в окне Tauri НЕ нажималась: среда не содержит WebKit/GTK dependencies и Xvfb. Установка проверена через production backend, а не через native GUI. Изменение базового шаблона среды требуется для честного native-button E2E. Windows/GPU/MT5 не проверены. Реальных сделок не было.

## Реальные запросы локальной модели
- Настоящий ping вернул READY: 17167 / 12968 / 12243 / 11463 мс в отдельных запусках.
- Без прогрева observer запрос не уложился в production deadline 28 с.
- После прогрева короткий smoke prompt получил валидный WAIT JSON от Qwen.
- С production observer system prompt запрос снова не уложился в 28 с даже после прогрева. Лог Ollama показывает успешную загрузку model runner примерно за 9.8 с и отмену /api/chat на 28.0018 с.
Вывод: в этой CPU-среде сервер поднимается и модель установлена, но deadline анализа может быть недостаточен. Это не доказательство такого же ограничения на GPU пользователя. Срок не увеличен: свежесть котировки и fail-closed политика не должны обходиться.

## Настоящий OpenRouter
Использован защищённо привязанный AEGIS_OPENROUTER_API_KEY, код production consult и фиксированная anthropic/claude-fable-5. Ключ не записан в файлы. Выполнены два фактических облачных запроса (оплачиваются провайдером).
1. Короткий smoke system prompt + реальный local WAIT proposal: облако вернуло WAIT в Markdown fenced JSON. Строгий production parser отклонил его, действие не принято. Это не следует считать успешным end-to-end согласованием.
2. Точный production observer system prompt + явно синтетическое local WAIT proposal: облако вернуло сырой JSON, schema parser принял WAIT, snapshot_id=42, отсутствующие price/bars/trend обозначены как неуспешные checks. Сопоставление action/snapshot прошло. В этом тесте локальное предложение — fixture, не ответ реальной модели.
Полностью успешная цепочка production Qwen → production cloud → full observer validation НЕ подтверждена из-за local CPU timeout. Native event loop/реальная котировка не запускались.

## Логика согласования
Облако получает snapshot и local_proposal: это review, не независимое слепое голосование и не свободный чат двух ИИ. При несовпадении action/snapshot/position/price/fraction решение отклоняется. Затем облачный результат проверяется с актуальной котировкой и локальный результат также повторно валидируется. Ключ/ошибка/timeout/неверная schema/небезопасные цены не должны приводить к новым входам.

## Найдены и исправлены три дефекта
1. Cloud content принимался при finish_reason=error. Теперь требуется stop и отсутствие error envelope.
2. Local content принимался при done=false. Теперь observe требует done=true и done_reason=stop.
3. Desktop compaction удалял context_candles и 20 закрытых баров data-стратегии. Общий tested helper теперь сохраняет уже ограниченную evidence для обоих providers.
Добавлены 15 regression tests. Helper перенесён перед Rust test module для Clippy.

## Проверки HTTP fixtures (не настоящие модели)
Matching long/wait проходят acceptance gates; disagreement, неверный snapshot, небезопасные prices, malformed JSON/schema, stale quote, bad timeframe, excessive spread, HTTP errors, empty/truncated content, missing key и timeout отклоняются. Production transport использован с loopback HTTP fixtures. Полный Tauri loop не запускался; agreement predicate проверен тестом на совпадение с app source.

## Итоговая локальная проверка
cargo test -p aegis-core: 142 unit + 6 bounce integration + 3 venue integration = 151 успешно; 0 failures. core Clippy all-targets -D warnings, cargo fmt и git diff --check проходят.

## Что не стоит делать
Не ослаблять строгий JSON parser, не принимать incomplete response, не включать Money для smoke test. Перед native release нужны окно Tauri/кнопка установки, повторные Start/Stop/Ping, production snapshot с Qwen в deadline, валидный provider review и demo acceptance. Реальный CPU timeout остаётся диагностированным ограничением; GPU performance и полноценное согласование не сертифицированы.
