"""Generates src/vfx_schema.rs and src/gpu/vfx_consts.wgsl from VTS Files/EffectConfigs.cs (parsed JSON)."""
import json, re, sys

src, root = sys.argv[1], sys.argv[2]
cfgs = json.load(open(src))

EFFECT_RU = {
    "ColorGrading": "Градация цвета", "WeatherEffects": "Погодные эффекты", "Bloom": "Свечение и световые эффекты",
    "Backlight": "Задняя подсветка и обводка", "CustomParticles": "Пользовательские эффекты частиц", "BackgroundShift": "Сдвиг фона",
    "SimpleOverlay": "Простое наложение", "Vignette": "Виньетка", "ChromaticAberration": "Хроматическая аберрация",
    "OldFilm": "Старая плёнка", "Lowfps": "Низкий FPS", "Datamosh": "Имитация битого видеофайла", "LineScanner": "Линейное сканирование",
    "ParticleShower": "Падающие частицы", "AnalogGlitch": "Аналоговые помехи", "DigitalGlitch": "Цифровые помехи",
    "Letterbox": "Кинематографическая рамка", "FoggyWindow": "Запотевшее окно", "Speedlines": "Линии скорости",
    "Pixelation": "Пикселизация", "LensDistortion": "Искажение линзы", "WaveDistortion": "Волновое искажение",
    "BlurEffects": "Размытие", "Grain": "Зернистость ТВ-помех", "Vhs": "Артефакты видеоплёнки / старый телевизор",
    "Outline": "Контурная обводка", "Posterize": "Постеризация", "Ascii": "ASCII-графика", "ModelGlitch": "Глитч модели",
}
EFFECT_EN = {
    "ColorGrading": "Color grading", "WeatherEffects": "Weather effects", "Bloom": "Bloom and light effects", "Backlight": "Backlight and outline",
    "CustomParticles": "Custom particle effects", "BackgroundShift": "Background shift", "SimpleOverlay": "Simple overlay", "Vignette": "Vignette",
    "ChromaticAberration": "Chromatic aberration", "OldFilm": "Old film", "Lowfps": "Low FPS", "Datamosh": "Datamosh (broken video file)",
    "LineScanner": "Line scanner", "ParticleShower": "Particle shower", "AnalogGlitch": "Analog glitch", "DigitalGlitch": "Digital glitch",
    "Letterbox": "Letterbox", "FoggyWindow": "Foggy window", "Speedlines": "Speedlines", "Pixelation": "Pixelation", "LensDistortion": "Lens distortion",
    "WaveDistortion": "Wave distortion", "BlurEffects": "Blur effects", "Grain": "Grain", "Vhs": "VHS / old TV", "Outline": "Outline",
    "Posterize": "Posterize", "Ascii": "ASCII", "ModelGlitch": "Model glitch",
}
# Effects that move/distort pixels (break item pinning visually) = VTS "restricted" candidates.
DISTORTS = {"Datamosh", "AnalogGlitch", "DigitalGlitch", "LensDistortion", "WaveDistortion", "ModelGlitch", "Vhs", "BackgroundShift"}
RESTRICTED = {"ModelGlitch"}

TAG_RU = {"All": "Все", "Sparkle": "Искры", "Floaty particles": "Парящие частицы", "Fog": "Туман", "Light spheres": "Световые сферы",
          "Hearts": "Сердечки", "Custom 1": "Своё 1", "Custom 2": "Своё 2", "Custom 3": "Своё 3"}
PHRASE_RU = {
    "Effect on/off": "Эффект вкл./выкл.", "Effect on/off - Multiplier for opacity": "Эффект вкл./выкл. — множитель прозрачности",
    "Hue shift": "Сдвиг оттенка", "Saturation": "Насыщенность", "Brightness": "Яркость", "Contrast": "Контраст", "Color filter": "Цветовой фильтр",
    "Whitebalance temperature": "Баланс белого: температура", "Whitebalance tint": "Баланс белого: оттенок", "Invert color": "Инверсия цвета",
    "Rain strength": "Сила дождя", "Snow strength": "Сила снега", "Rain  -  In front": "Дождь — перед моделью", "Snow  -  In front": "Снег — перед моделью",
    "Model color darken": "Затемнение модели", "Background color darken": "Затемнение фона", "Bloom threshold": "Порог свечения",
    "Bloom intensity": "Интенсивность свечения", "Bloom tint color": "Цвет свечения", "Light streak threshold": "Порог световых лучей",
    "Light streak intensity": "Интенсивность лучей", "Light streak tint color": "Цвет лучей", "Light streak vertical": "Вертикальные лучи",
    "Microphone effect - Microphone volume boosts strength": "Эффект микрофона — громкость усиливает эффект",
    "Quality - Bloom quality": "Качество свечения", "Blur BG overlay - For Model": "Размытый фон поверх — для модели",
    "Blur BG overlay - For Background": "Размытый фон поверх — для фона", "Backlight strength - Main": "Сила подсветки — основная",
    "Backlight strength - Directional": "Сила подсветки — направленная", "Backlight brightness limit": "Ограничение яркости подсветки",
    "Backlight direction": "Направление подсветки", "Backlight dir. both sides": "Подсветка с обеих сторон", "Backlight softness": "Мягкость подсветки",
    "Backlight color tint": "Цвет подсветки", "Backlight col. from background": "Цвет подсветки от фона", "Outline size": "Толщина обводки",
    "Outline color": "Цвет обводки", "Outline stripe color": "Цвет полос обводки", "Outline stripe count": "Количество полос",
    "Outline stripe speed": "Скорость полос", "Outline stripe curve": "Изгиб полос", "Shadow color": "Цвет тени", "Shadow offset X": "Смещение тени X",
    "Shadow offset Y": "Смещение тени Y", "Move with head movement": "Двигаться с головой", "Amount": "Количество", "Size": "Размер",
    "Color A": "Цвет A", "Color B": "Цвет B", "Texture file": "Файл текстуры", "Additive Particles - Will make particles shiny": "Аддитивные частицы — блестят",
    "Show behind model": "За моделью", "Fill to center": "Заполнять к центру", "Base Rotation": "Базовый поворот", "Rotation Speed": "Скорость вращения",
    "Microphone effect - Microphone volume moves particles faster": "Эффект микрофона — громкость ускоряет частицы",
    "Zoom in": "Приближение", "Microphone effect - Microphone volume zooms in": "Эффект микрофона — громкость приближает",
    "Move from X tracking": "Сдвиг от трекинга X", "Move from Y tracking": "Сдвиг от трекинга Y", "Tracking movement smoothing": "Сглаживание движения",
    "Move X randomly": "Случайный сдвиг X", "Move Y randomly": "Случайный сдвиг Y", "Rotate randomly": "Случайный поворот",
    "Random movement speed": "Скорость случайного движения", "Blur back visibility": "Видимость заднего размытия", "Blur back strength": "Сила заднего размытия",
    "Blur back brightness": "Яркость заднего размытия", "Blur front visibility": "Видимость переднего размытия", "Blur front strength": "Сила переднего размытия",
    "Blur front brightness": "Яркость переднего размытия", "Overlay image file": "Файл изображения наложения", "Tint color": "Цвет тонирования",
    "Smoothness": "Мягкость", "Color": "Цвет", "Center X": "Центр X", "Center Y": "Центр Y", "Roundness": "Округлость", "Even side length": "Равные стороны",
    "Blur edges": "Размытие краёв", "FPS limit": "Ограничение FPS", "Film burn": "Прожиг плёнки", "Film dirt size": "Размер грязи на плёнке",
    "FPS jitter strength": "Дрожание FPS", "Screen tearing": "Разрыв кадра", "Reset after seconds": "Сброс через (сек.)", "Entropy": "Энтропия",
    "Noise Contrast": "Контраст шума", "Velocity Scale": "Масштаб скорости", "Diffusion": "Диффузия", "Direction": "Направление",
    "Total scan steps": "Всего шагов сканирования", "Scan step size": "Размер шага", "Scan line color": "Цвет линии", "Scan line size": "Толщина линии",
    "Wait between scans - Reset after seconds": "Пауза между проходами (сек.)", "Fall speed": "Скорость падения", "Scanline Jitter": "Дрожание строк",
    "Vertical jump": "Вертикальные скачки", "Horizontal shake": "Горизонтальная тряска", "Color Drift": "Дрейф цвета",
    "Strength - Strength of the effect": "Сила эффекта", "Color Shift - How much to shift the color for this effect?": "Сдвиг цвета",
    "Letterbox Y": "Полосы по Y", "Letterbox X": "Полосы по X", "Fog amount": "Количество тумана", "Fog color tint": "Цвет тумана",
    "Fog brightness boost": "Яркость тумана", "Raindrop visibility": "Видимость капель", "Raindrop speed": "Скорость капель", "Raindrop size": "Размер капель",
    "Fog wipe size": "Размер протирания", "Fog wipe seconds": "Время до запотевания (сек.)", "Wiped fog stays wiped": "Протёртое не запотевает",
    "X center": "Центр X", "Y center": "Центр Y", "Speedlines color A": "Цвет линий A", "Speedlines color B": "Цвет линий B",
    "Resolution": "Разрешение", "Color  1": "Цвет 1", "Color  2": "Цвет 2", "Color  3": "Цвет 3", "Color  4": "Цвет 4", "Color  5": "Цвет 5",
    "Color  6": "Цвет 6", "Color  7": "Цвет 7", "Color  8": "Цвет 8", "Fry": "Пережаренность", "Lens strength": "Сила линзы", "Squish": "Сжатие",
    "Heat wave strength": "Сила марева", "Raindrop strength": "Сила капель", "Raindrop interval (sec)": "Интервал капель (сек.)",
    "Wave strength X": "Сила волны X", "Wave scroll X": "Прокрутка волны X", "Wave frequency X": "Частота волны X", "Wave strength Y": "Сила волны Y",
    "Wave scroll Y": "Прокрутка волны Y", "Wave frequency Y": "Частота волны Y", "Blur visibility": "Видимость размытия", "Blur strength": "Сила размытия",
    "Pixelation": "Пикселизация", "Motion blur": "Размытие движения", "Luminosity": "Яркость зерна", "Colored": "Цветное",
    "Monitor fisheye": "Выпуклость экрана", "Vignette": "Виньетка", "Sideways blur": "Боковое размытие", "Grain noise": "Шум зерна",
    "Line noise": "Шум строк", "Horizontal twitch (up/down)": "Дёргание вверх/вниз", "Vertical twitch (left/right)": "Дёргание влево/вправо",
    "Interlacing": "Чересстрочность", "Gamma correction": "Гамма-коррекция", "Pale color": "Бледность", "Afterimage strength": "Сила остаточного изображения",
    "Afterimage color": "Цвет остаточного изображения", "Sharpen image": "Резкость", "Outline visibility": "Видимость контура", "Outline threshold": "Порог контура",
    "Outline contrast": "Контраст контура", "Character visibility": "Видимость символов", "Color overlay strength": "Сила цветного наложения",
    "Color overlay": "Цветное наложение", "Explode model parts": "Разлёт частей модели", "Wiggle model parts": "Дрожание частей модели",
    "Pulsate model parts": "Пульсация частей модели",
}

def ru(expl):
    m = re.match(r"\[(.+?)\] (.*)", expl)
    if m:
        return f"[{TAG_RU.get(m.group(1), m.group(1))}] {PHRASE_RU.get(m.group(2), m.group(2))}"
    return PHRASE_RU.get(expl, expl)

def snake(s):
    s = re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", s)
    return s.lower()

def rs(s):
    return '"' + s.replace('\\', '\\\\').replace('"', '\\"') + '"'

effects = []
for c in cfgs:
    if c["effect"] not in [e for e in effects]:
        effects.append(c["effect"])

missing = [c["expl"] for c in cfgs if ru(c["expl"]) == c["expl"] and not re.match(r"\[", c["expl"])]
if missing:
    print("untranslated:", missing)

out = ["//! Generated from VTube Studio Files/EffectConfigs.cs by gen_vfx.py — do not edit by hand.",
       "//! VTS post-processing effects and their configs (enum IDs, ranges, defaults).",
       "", "use super::vfx::{ConfDef, Def, Kind};", "",
       "pub static DEFS: &[Def] = &["]
wgsl = ["// Generated by gen_vfx.py: per-effect parameter offsets into P.v (floats).", ""]
for e in effects:
    cs = [c for c in cfgs if c["effect"] == e]
    out.append(f"    Def {{ id: {rs(e)}, internal: {rs(snake(e))}, name_ru: {rs(EFFECT_RU[e])}, name_en: {rs(EFFECT_EN[e])}, distorts: {str(e in DISTORTS).lower()}, restricted: {str(e in RESTRICTED).lower()}, configs: &[")
    off = 0
    for c in cs:
        t = c["type"]
        kind = {"Float": "Kind::Float", "Int": "Kind::Int", "Bool": "Kind::Bool", "Color": "Kind::Color", "String": "Kind::Text", "SceneItem": "Kind::Item"}[t]
        mn, mx = c.get("min", 0.0), c.get("max", 0.0)
        d = c.get("default", "")
        if t in ("Float", "Int"):
            dv = f"{float(d)}"
            dstr, dcol, dalpha = "0.0", '""', "false"
        elif t == "Bool":
            dv = "1.0" if d.strip().lower() == "true" else "0.0"
            dstr, dcol, dalpha = "0.0", '""', "false"
        elif t == "Color":
            m = re.match(r"([0-9A-Fa-f]{6})\s*alpha:\s*(\w+)", d)
            dv, dcol, dalpha = "0.0", rs(m.group(1).upper()), ("true" if m.group(2) == "True" else "false")
        else:
            dv, dcol, dalpha = "0.0", '""', "false"
        size = 4 if t == "Color" else (0 if t in ("String", "SceneItem") else 1)
        cid = c["config"]
        out.append(f"        ConfDef {{ id: {rs(e + '_' + cid)}, internal: {rs(snake(e) + '-' + snake(cid))}, kind: {kind}, min: {float(mn)}, max: {float(mx)}, default: {dv}, color: {dcol}, alpha: {dalpha}, active: {str(c['active']).lower()}, en: {rs(c['expl'])}, ru: {rs(ru(c['expl']))}, offset: {off} }},")
        if size:
            wgsl.append(f"const {snake(e).upper()}_{snake(cid).upper()}: u32 = {off}u;")
        off += size
    out.append("    ] },")
    wgsl.append(f"// {e}: {off} floats")
out.append("];")
open(root + "/src/vfx_schema.rs", "w", encoding="utf-8").write("\n".join(out) + "\n")
open(root + "/src/gpu/vfx_consts.wgsl", "w", encoding="utf-8").write("\n".join(wgsl) + "\n")
print("effects", len(effects), "configs", len(cfgs))
