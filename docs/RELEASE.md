# Як випустити бету OptimaX на GitHub

Результат: на сторінці **Releases** репозиторію `VoroN5k/OptimaX` лежать два файли:
- `OptimaX-<версія>-win-x64-setup.exe` — інсталятор (NSIS): Program Files, ярлик у «Пуск»,
  видалення через «Програми й компоненти»;
- `OptimaX-<версія>-win-x64.zip` — портативна версія: розпакував і запустив.

Усередині обох — той самий `optimax.exe`.

Що всередині `optimax.exe`:
- увесь інтерфейс (Svelte-збірка вбудовується в exe);
- PresentMon 2.6.0 (вбудовано; при першому запуску розпаковується в
  `%LOCALAPPDATA%\OptimaX\bin` і перед кожним запуском перевіряється за SHA-256);
- маніфест `requireAdministrator` — Windows щоразу питає UAC.

Що потрібно на ПК друга: Windows 10/11 x64 і **WebView2** (на Win11 вбудований, на Win10
майже завжди є; якщо немає — програма сама скаже й запропонує офіційний інсталятор).
Інсталятор WebView2 сам **не** ставить (`webviewInstallMode: skip`): за CLAUDE.md 2.1 —
лише за згодою користувача, а це вже робить програма при старті.

---

## 0. Один раз: підготовка

**Інструменти на твоєму ПК** (у тебе вже є): Rust ≥ 1.95 з MSVC (`rustc --version`),
Visual Studio Build Tools з «Desktop development with C++», Node.js 22 (`node --version`).

**Доступ друзів до релізу.** Releases приватного репозиторію бачать лише ті, хто має до нього
доступ. Два варіанти:
- зробити репозиторій публічним (Settings → General → Danger Zone → Change visibility);
- лишити приватним і додати друзів (Settings → Collaborators → Add people) — тоді в кожного
  має бути акаунт GitHub.

**Сторінка репозиторію** — `README.md` (що це, вимоги, посилання на Releases). Перед
релізом перевір, що опис «Що вміє» відповідає тому, що реально є в збірці.

---

## 1. Номер версії

Версію програми задано в трьох місцях, вони мають збігатися:
- `Cargo.toml` (корінь) → `[workspace.package] version`;
- `crates/app/tauri.conf.json` → `"version"`;
- `ui/package.json` → `"version"`.

Для бет простіше лишити в цих файлах числову версію (`0.1.0` — її видно внизу сайдбара),
а позначку «бета» давати тегом і назвою релізу: `v0.1.0-beta.1`, `v0.1.0-beta.2`, …
Коли бета стане стабільною — `v0.1.0` без суфікса; наступна серія — версія `0.2.0`
у трьох файлах і теги `v0.2.0-beta.N`.

> Версію з суфіксом (`0.1.0-beta.1`) прямо в файлах ще не перевіряли зі збіркою Tauri
> (Windows записує у властивості exe лише числа). Якщо захочеш — спершу пробна збірка.

---

## 2. Перевірки перед збіркою

З кореня репозиторію (PowerShell):

```powershell
git switch main
git pull --ff-only
git status            # має бути чисто

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd ui
npm ci                # чисте встановлення за package-lock.json
npm run check
npm run check:i18n
cd ..
```

Усе зелене — їдемо далі. Ні — реліз не робимо.

---

## 3. CHANGELOG

У `CHANGELOG.md` перейменуй розділ `## [Unreleased]` на версію з датою й додай над ним
новий порожній `Unreleased`:

```markdown
## [Unreleased]

## [0.1.0-beta.1] - 2026-10-30

### Додано
...
```

Текст із цього розділу піде в опис релізу (крок 6). Закоміть зміну:

```powershell
git add CHANGELOG.md
git commit -m "chore: release 0.1.0-beta.1"
git push
```

---

## 4. Збірка

```powershell
cd ui
npm run app:bundle
cd ..
```

`app:bundle` — це `tauri build --bundles nsis`: Tauri сам запускає `npm run build`
(фронтенд), потім `cargo build --release`, потім NSIS. Результат:

```
target\release\optimax.exe                                   портативний exe (≈ 15 МБ)
target\release\bundle\nsis\OptimaX_0.1.0_x64-setup.exe        інсталятор (≈ 3,7 МБ)
```

Повторна збірка — ≈ 1,5 хв. Налаштування інсталятора — `bundle` у
`crates/app/tauri.conf.json`: лише NSIS (MSI свідомо не робимо), встановлення для всіх
користувачів у Program Files, мови українська й англійська, `THIRD_PARTY_LICENSES.txt`
кладеться поруч з exe.

> **Перша збірка інсталятора** завантажує NSIS і плагін `nsis_tauri_utils` з
> `github.com/tauri-apps` (Tauri перевіряє їхній хеш) і кешує їх у `%LOCALAPPDATA%\tauri`.
> Потрібен інтернет; далі збирається офлайн.
>
> Лише exe без інсталятора: `npm run app:build` (`tauri build --no-bundle`).

**Швидка перевірка перед публікацією** (обов'язково, це вже не dev-режим):
1. Запусти `target\release\optimax.exe` → має з'явитися запит UAC → заставка → «Аналіз».
2. Пройди «Діагностику» з будь-якою грою (перевіряє вбудований PresentMon).
3. «Очищення» → скан (лише скан, без видалення).
4. Закрий програму.
5. Запусти `OptimaX_0.1.0_x64-setup.exe` → встанови → запусти з меню «Пуск» → закрий →
   видали через «Параметри → Програми». Тека `C:\Program Files\OptimaX` має зникнути.

---

## 5. Файли релізу й контрольні суми

```powershell
$v = "0.1.0-beta.1"
$dir = "release\OptimaX-$v"
New-Item -ItemType Directory -Force $dir | Out-Null
Copy-Item target\release\optimax.exe "$dir\OptimaX.exe"
Copy-Item THIRD_PARTY_LICENSES "$dir\THIRD_PARTY_LICENSES.txt"
Compress-Archive -Path "$dir\*" -DestinationPath "release\OptimaX-$v-win-x64.zip" -Force
Copy-Item target\release\bundle\nsis\OptimaX_*_x64-setup.exe "release\OptimaX-$v-win-x64-setup.exe"
Get-FileHash "release\OptimaX-$v-win-x64-setup.exe", "release\OptimaX-$v-win-x64.zip" -Algorithm SHA256
```

- `THIRD_PARTY_LICENSES` **обов'язково** кладемо поруч: ліцензія PresentMon (MIT) і шрифтів
  (OFL) вимагає передавати їхній текст разом із програмою (в інсталяторі це вже налаштовано).
- Обидва SHA-256 з останньої команди скопіюй в опис релізу — друзі зможуть звірити файли.
- Тека `release\` уже в `.gitignore` — у git вона не потрапить.

---

## 6. Тег і реліз на GitHub

**Тег** (позначає коміт, з якого зібрано exe):

```powershell
git tag -a v0.1.0-beta.1 -m "OptimaX 0.1.0-beta.1"
git push origin v0.1.0-beta.1
```

**Реліз через сайт:**
1. `https://github.com/VoroN5k/OptimaX/releases` → **Draft a new release**.
2. **Choose a tag** → `v0.1.0-beta.1`.
3. **Release title**: `OptimaX 0.1.0 beta 1`.
4. Опис — шаблон нижче.
5. Перетягни `release\OptimaX-0.1.0-beta.1-win-x64-setup.exe` і
   `release\OptimaX-0.1.0-beta.1-win-x64.zip` у поле **Attach binaries**.
6. Постав галочку **Set as a pre-release** (GitHub позначить реліз як бету).
7. **Publish release**.

Посилання для друзів: `https://github.com/VoroN5k/OptimaX/releases/latest` веде лише на
стабільні релізи, тож для бет давай пряме посилання на реліз:
`https://github.com/VoroN5k/OptimaX/releases/tag/v0.1.0-beta.1`.

**Шаблон опису:**

```markdown
Бета-версія: можливі помилки. Відгуки — в Issues або напряму мені.

### Що потрібно
- Windows 10/11 x64
- WebView2 (на Win11 є; на Win10 програма підкаже, якщо немає)
- права адміністратора (Windows спитає при запуску)

### Як встановити
- **Інсталятор:** завантаж `OptimaX-0.1.0-beta.1-win-x64-setup.exe` і запусти.
- **Без встановлення:** завантаж `OptimaX-0.1.0-beta.1-win-x64.zip`, розпакуй у будь-яку
  теку, запусти `OptimaX.exe`.

Windows може показати «Windows захистив ваш ПК» — програма не підписана сертифікатом.
Натисни «Докладніше» → «Однаково запустити».

### Що нового
(вставити розділ з CHANGELOG.md)

### SHA-256
- `OptimaX-0.1.0-beta.1-win-x64-setup.exe`: `<хеш із кроку 5>`
- `OptimaX-0.1.0-beta.1-win-x64.zip`: `<хеш із кроку 5>`
```

> Через GitHub CLI (необов'язково): `winget install GitHub.cli`, `gh auth login`, далі
> `gh release create v0.1.0-beta.1 release\OptimaX-0.1.0-beta.1-win-x64-setup.exe release\OptimaX-0.1.0-beta.1-win-x64.zip --prerelease --title "OptimaX 0.1.0 beta 1" --notes-file release\notes.md`.

---

## 7. Що варто знати перед тим, як роздати

- **SmartScreen.** Exe без цифрового підпису, тож Windows попереджає при першому запуску.
  Це нормально для бет; прибирається лише платним сертифікатом підпису коду.
- **Антивіруси.** Програма з правами адміністратора, яка розпаковує й запускає вбудований
  exe (PresentMon), часом викликає хибне спрацювання евристики. Якщо в когось трапиться —
  хай надішле назву антивіруса й текст попередження; не радь друзям вимикати захист.
- **Дані програми** лежать у `%LOCALAPPDATA%\OptimaX` (журнали, карантин, налаштування).
  Деінсталятор їх **не** видаляє (його галочка «видалити дані» прибирає лише
  `%LOCALAPPDATA%\com.x44to.optimax` — дані WebView2). Так безпечніше: у карантині можуть
  бути файли, які людина ще захоче відновити. Повністю прибрати = видалити програму, потім
  цю теку (спершу відновити потрібне з карантину).
- **Ліцензія самого OptimaX.** У репозиторії немає файлу `LICENSE` — тобто юридично всі
  права твої й код ніхто не може використовувати. Для роздачі exe друзям це не заважає;
  якщо репозиторій буде публічним і ти хочеш дозволити використання коду — додай ліцензію.
- **Скриншот** у README зробить сторінку репозиторію зрозумілішою (поки його немає).

---

## 8. Наступна бета

1. Зміни в `main`, записи в `## [Unreleased]` у CHANGELOG.
2. Кроки 2–6 з новим номером: `v0.1.0-beta.2`.
3. Старі бети можна не видаляти — GitHub покаже новішу першою.
