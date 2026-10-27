# Aural

Player de música nativo para Linux, escrito em Rust com `egui`. O Aural nasceu
como uma implementação independente inspirada no conjunto de recursos de players
móveis modernos, mas sem reutilizar código, assets, textos ou a identidade visual
de nenhum aplicativo existente.

O foco atual é uma biblioteca local combinada com servidores
**OpenSubsonic/Navidrome**, em uma interface em português.

## Estado atual (MVP)

- Escaneamento de pastas locais com metadados via `lofty`.
- Formatos indexados: FLAC, MP3, M4A/M4B/MP4, OGG/OGA, WAV e AAC.
- Extração e cache de capas incorporadas.
- Navegação por álbuns, artistas e busca local.
- Fila de reprodução, faixa anterior/próxima, play/pause, seek e volume.
- Avanço automático para a próxima faixa da fila.
- Reprodução local com `rodio`: FLAC, MP3, AAC/MP4, Vorbis e WAV conforme os
  decodificadores habilitados por padrão.
- Integração **MPRIS**: metadados, capa, status, volume e comandos de mídia.
- Cliente **OpenSubsonic**: autenticação com salt + token, listagem de álbuns,
  detalhes de álbum, capa e download da faixa para reprodução.
- Interface em português, tema escuro e controles desenhados sem depender de
  glifos extras de fonte.

## Limitações conhecidas

- Faixas remotas são baixadas inteiras para a memória antes de tocar; ainda não
  há cache de streaming, buffer progressivo ou reprodução gapless.
- Não há suporte a DSD, APE, WavPack ou Opus nesta versão.
- Não há listas de reprodução, letras, ReplayGain, equalizador, transcodificação,
  Chromecast, MPRIS position updates contínuos ou integração com provedores em
  nuvem.
- A senha do servidor é gravada em texto puro no arquivo de configuração, com
  permissão `0600`. Use HTTPS e proteja o arquivo.
- O scanner é manual depois da primeira execução. Se a biblioteca estiver vazia
  e houver uma pasta configurada, ele roda automaticamente na inicialização.

## Dependências de sistema

Além do toolchain Rust (edição 2024), o build precisa de ALSA, D-Bus e das
bibliotecas de janela/OpenGL usadas pelo `eframe`.

Em Debian/Ubuntu, uma lista típica é:

```bash
sudo apt install build-essential pkg-config \
  libasound2-dev libdbus-1-dev \
  libx11-dev libxi-dev libxcursor-dev libxrandr-dev \
  libxkbcommon-dev libwayland-dev libgl1-mesa-dev
```

Os nomes variam em outras distribuições. O crate `rusqlite` é compilado com
SQLite embutido, então não é necessário instalar o SQLite do sistema.

## Compilar e executar

```bash
git clone https://github.com/rugalo798/aural.git
cd aural
cargo build --release
cargo run --release
```

Na primeira execução, abra **Configurações** para adicionar uma pasta local ou
um servidor OpenSubsonic. A configuração fica em:

- `~/.config/aural/config.toml`
- `~/.local/share/aural/library.db`
- `~/.cache/aural/covers/`

Respeitando `XDG_CONFIG_HOME`, `XDG_DATA_HOME` e `XDG_CACHE_HOME`.

### Servidor OpenSubsonic/Navidrome

Informe a URL base (por exemplo, `https://musica.exemplo.com`), usuário e senha.
O cliente usa os endpoints `ping`, `getAlbumList2`, `getAlbum`, `getCoverArt` e
`stream` com autenticação padrão do Subsonic.

## Testes

```bash
cargo test
```

O teste de scanner só executa quando a variável `AURAL_TEST_MUSIC` aponta para
uma pasta com arquivos de áudio:

```bash
AURAL_TEST_MUSIC=/caminho/da/musica cargo test
```

Também há um teste do cliente Subsonic contra um servidor HTTP local simulado,
que valida a geração do token, o parsing das respostas e a montagem das URLs de
stream e capa.

## Organização do código

- `src/app.rs`: estado da aplicação, interface `egui`, fila e integração MPRIS.
- `src/audio.rs`: reprodução e controle de posição/volume com `rodio`.
- `src/scan.rs`: caminhada pelo sistema de arquivos, leitura de tags e capas.
- `src/db.rs`: schema SQLite e consultas da biblioteca.
- `src/subsonic.rs`: cliente OpenSubsonic.
- `src/config.rs`: configuração TOML e diretórios XDG.
- `src/model.rs`: modelos de faixa, álbum e fonte.

## Escopo e independência

Este projeto é uma reimplementação independente. Ele não contém código
decompilado, recursos, ícones, fontes, textos ou marcas de terceiros. Nomes de
protocolos e recursos de interoperabilidade são usados apenas para descrever
compatibilidade.

## Licença

Copyright (C) 2026 Aural contributors.

Este projeto é distribuído sob a **GNU General Public License, versão 3.0**
(`GPL-3.0-only`). O texto completo está em [LICENSE](LICENSE).

## Próximos passos

- Busca remota no servidor e navegação por artistas remotos.
- Cache de streaming e reprodução gapless.
- Listas de reprodução e persistência da fila.
- Integração mais completa com MPRIS e teclas de mídia.
- Suporte a mais formatos e saídas de áudio.
