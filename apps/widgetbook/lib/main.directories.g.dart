// dart format width=80
// coverage:ignore-file
// ignore_for_file: type=lint
// ignore_for_file: unused_import, prefer_relative_imports, directives_ordering

// GENERATED CODE - DO NOT MODIFY BY HAND

// **************************************************************************
// AppGenerator
// **************************************************************************

// ignore_for_file: no_leading_underscores_for_library_prefixes

import 'package:storm_widgetbook/index/usage_index.dart'
    as _storm_widgetbook_index_usage_index;
import 'package:storm_widgetbook/use_cases/accent_picker.dart'
    as _storm_widgetbook_use_cases_accent_picker;
import 'package:storm_widgetbook/use_cases/brand_mark.dart'
    as _storm_widgetbook_use_cases_brand_mark;
import 'package:storm_widgetbook/use_cases/breadcrumb.dart'
    as _storm_widgetbook_use_cases_breadcrumb;
import 'package:storm_widgetbook/use_cases/choice_cards.dart'
    as _storm_widgetbook_use_cases_choice_cards;
import 'package:storm_widgetbook/use_cases/choice_chips.dart'
    as _storm_widgetbook_use_cases_choice_chips;
import 'package:storm_widgetbook/use_cases/code_block.dart'
    as _storm_widgetbook_use_cases_code_block;
import 'package:storm_widgetbook/use_cases/count_badge.dart'
    as _storm_widgetbook_use_cases_count_badge;
import 'package:storm_widgetbook/use_cases/folder_row.dart'
    as _storm_widgetbook_use_cases_folder_row;
import 'package:storm_widgetbook/use_cases/hairline.dart'
    as _storm_widgetbook_use_cases_hairline;
import 'package:storm_widgetbook/use_cases/image_thumbnail.dart'
    as _storm_widgetbook_use_cases_image_thumbnail;
import 'package:storm_widgetbook/use_cases/inbox_header.dart'
    as _storm_widgetbook_use_cases_inbox_header;
import 'package:storm_widgetbook/use_cases/inline_confirm.dart'
    as _storm_widgetbook_use_cases_inline_confirm;
import 'package:storm_widgetbook/use_cases/key_value_list.dart'
    as _storm_widgetbook_use_cases_key_value_list;
import 'package:storm_widgetbook/use_cases/kind_badge.dart'
    as _storm_widgetbook_use_cases_kind_badge;
import 'package:storm_widgetbook/use_cases/note_row.dart'
    as _storm_widgetbook_use_cases_note_row;
import 'package:storm_widgetbook/use_cases/notice_banner.dart'
    as _storm_widgetbook_use_cases_notice_banner;
import 'package:storm_widgetbook/use_cases/notification_row.dart'
    as _storm_widgetbook_use_cases_notification_row;
import 'package:storm_widgetbook/use_cases/numbered_steps.dart'
    as _storm_widgetbook_use_cases_numbered_steps;
import 'package:storm_widgetbook/use_cases/outline_tag.dart'
    as _storm_widgetbook_use_cases_outline_tag;
import 'package:storm_widgetbook/use_cases/panels.dart'
    as _storm_widgetbook_use_cases_panels;
import 'package:storm_widgetbook/use_cases/popover.dart'
    as _storm_widgetbook_use_cases_popover;
import 'package:storm_widgetbook/use_cases/reading_measure.dart'
    as _storm_widgetbook_use_cases_reading_measure;
import 'package:storm_widgetbook/use_cases/save_state_label.dart'
    as _storm_widgetbook_use_cases_save_state_label;
import 'package:storm_widgetbook/use_cases/section_label.dart'
    as _storm_widgetbook_use_cases_section_label;
import 'package:storm_widgetbook/use_cases/session_status.dart'
    as _storm_widgetbook_use_cases_session_status;
import 'package:storm_widgetbook/use_cases/settings_row.dart'
    as _storm_widgetbook_use_cases_settings_row;
import 'package:storm_widgetbook/use_cases/settings_widgets.dart'
    as _storm_widgetbook_use_cases_settings_widgets;
import 'package:storm_widgetbook/use_cases/sidebar_rows.dart'
    as _storm_widgetbook_use_cases_sidebar_rows;
import 'package:storm_widgetbook/use_cases/states.dart'
    as _storm_widgetbook_use_cases_states;
import 'package:storm_widgetbook/use_cases/status_dot.dart'
    as _storm_widgetbook_use_cases_status_dot;
import 'package:storm_widgetbook/use_cases/storm_button.dart'
    as _storm_widgetbook_use_cases_storm_button;
import 'package:storm_widgetbook/use_cases/storm_button_kind.dart'
    as _storm_widgetbook_use_cases_storm_button_kind;
import 'package:storm_widgetbook/use_cases/storm_card.dart'
    as _storm_widgetbook_use_cases_storm_card;
import 'package:storm_widgetbook/use_cases/storm_checkbox.dart'
    as _storm_widgetbook_use_cases_storm_checkbox;
import 'package:storm_widgetbook/use_cases/storm_checkbox_row.dart'
    as _storm_widgetbook_use_cases_storm_checkbox_row;
import 'package:storm_widgetbook/use_cases/storm_icon.dart'
    as _storm_widgetbook_use_cases_storm_icon;
import 'package:storm_widgetbook/use_cases/storm_input.dart'
    as _storm_widgetbook_use_cases_storm_input;
import 'package:storm_widgetbook/use_cases/storm_sheet.dart'
    as _storm_widgetbook_use_cases_storm_sheet;
import 'package:storm_widgetbook/use_cases/storm_toggle.dart'
    as _storm_widgetbook_use_cases_storm_toggle;
import 'package:storm_widgetbook/use_cases/tag_chip.dart'
    as _storm_widgetbook_use_cases_tag_chip;
import 'package:storm_widgetbook/use_cases/tag_group.dart'
    as _storm_widgetbook_use_cases_tag_group;
import 'package:storm_widgetbook/use_cases/tappable.dart'
    as _storm_widgetbook_use_cases_tappable;
import 'package:widgetbook/widgetbook.dart' as _widgetbook;

final directories = <_widgetbook.WidgetbookNode>[
  _widgetbook.WidgetbookCategory(
    name: 'Controls',
    children: [
      _widgetbook.WidgetbookComponent(
        name: 'AccentPicker',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder:
                _storm_widgetbook_use_cases_accent_picker.accentPickerDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'narrow (wraps)',
            builder:
                _storm_widgetbook_use_cases_accent_picker.accentPickerNarrow,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'none selected',
            builder: _storm_widgetbook_use_cases_accent_picker.accentPickerNone,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'AccentSwatch',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder:
                _storm_widgetbook_use_cases_accent_picker.accentSwatchDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'none (no colour)',
            builder: _storm_widgetbook_use_cases_accent_picker.accentSwatchNone,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'selected',
            builder:
                _storm_widgetbook_use_cases_accent_picker.accentSwatchSelected,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'ChoiceCards',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder:
                _storm_widgetbook_use_cases_choice_cards.choiceCardsDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'disabled',
            builder:
                _storm_widgetbook_use_cases_choice_cards.choiceCardsDisabled,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'empty',
            builder: _storm_widgetbook_use_cases_choice_cards.choiceCardsEmpty,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'error (alert)',
            builder: _storm_widgetbook_use_cases_choice_cards.choiceCardsError,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long text (overflow)',
            builder: _storm_widgetbook_use_cases_choice_cards.choiceCardsLong,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'one option disabled',
            builder: _storm_widgetbook_use_cases_choice_cards
                .choiceCardsOptionDisabled,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'ChoiceChips',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder:
                _storm_widgetbook_use_cases_choice_chips.choiceChipsDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'disabled',
            builder:
                _storm_widgetbook_use_cases_choice_chips.choiceChipsDisabled,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'empty',
            builder: _storm_widgetbook_use_cases_choice_chips.choiceChipsEmpty,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long labels (wrap)',
            builder: _storm_widgetbook_use_cases_choice_chips.choiceChipsLong,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'NumberedSteps',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder:
                _storm_widgetbook_use_cases_numbered_steps.numberedStepsDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'empty',
            builder:
                _storm_widgetbook_use_cases_numbered_steps.numberedStepsEmpty,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'inline action',
            builder:
                _storm_widgetbook_use_cases_numbered_steps.numberedStepsInline,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long text (overflow)',
            builder:
                _storm_widgetbook_use_cases_numbered_steps.numberedStepsLong,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'SettingsRow',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder:
                _storm_widgetbook_use_cases_settings_row.settingsRowDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'destructive',
            builder:
                _storm_widgetbook_use_cases_settings_row.settingsRowDestructive,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'disabled toggle',
            builder:
                _storm_widgetbook_use_cases_settings_row.settingsRowDisabled,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long text (overflow)',
            builder: _storm_widgetbook_use_cases_settings_row.settingsRowLong,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'muted',
            builder: _storm_widgetbook_use_cases_settings_row.settingsRowMuted,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'tappable with leading icon',
            builder:
                _storm_widgetbook_use_cases_settings_row.settingsRowTappable,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'with action',
            builder: _storm_widgetbook_use_cases_settings_row.settingsRowAction,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'StormButton',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'comfortable',
            builder:
                _storm_widgetbook_use_cases_storm_button.stormButtonComfortable,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'danger (error)',
            builder: _storm_widgetbook_use_cases_storm_button.stormButtonDanger,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder:
                _storm_widgetbook_use_cases_storm_button.stormButtonDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'disabled',
            builder:
                _storm_widgetbook_use_cases_storm_button.stormButtonDisabled,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long label (overflow)',
            builder:
                _storm_widgetbook_use_cases_storm_button.stormButtonLongLabel,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'outline',
            builder:
                _storm_widgetbook_use_cases_storm_button.stormButtonOutline,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'primary',
            builder:
                _storm_widgetbook_use_cases_storm_button.stormButtonPrimary,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'soft',
            builder: _storm_widgetbook_use_cases_storm_button.stormButtonSoft,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'text',
            builder: _storm_widgetbook_use_cases_storm_button.stormButtonText,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'StormButtonKind',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'colour override',
            builder: _storm_widgetbook_use_cases_storm_button_kind
                .stormButtonKindColour,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'danger text',
            builder: _storm_widgetbook_use_cases_storm_button_kind
                .stormButtonKindDangerText,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'StormCheckbox',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'checked',
            builder:
                _storm_widgetbook_use_cases_storm_checkbox.stormCheckboxChecked,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder:
                _storm_widgetbook_use_cases_storm_checkbox.stormCheckboxDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'disabled',
            builder: _storm_widgetbook_use_cases_storm_checkbox
                .stormCheckboxDisabled,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'StormCheckboxRow',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_storm_checkbox_row
                .stormCheckboxRowDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'disabled',
            builder: _storm_widgetbook_use_cases_storm_checkbox_row
                .stormCheckboxRowDisabled,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long text (overflow)',
            builder: _storm_widgetbook_use_cases_storm_checkbox_row
                .stormCheckboxRowLong,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'row (long text)',
            builder:
                _storm_widgetbook_use_cases_storm_checkbox.stormCheckboxRowLong,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'unchecked',
            builder: _storm_widgetbook_use_cases_storm_checkbox_row
                .stormCheckboxRowOff,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'StormInput',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_storm_input.stormInputDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'disabled',
            builder: _storm_widgetbook_use_cases_storm_input.stormInputDisabled,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long text (overflow)',
            builder: _storm_widgetbook_use_cases_storm_input.stormInputLong,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'multiline',
            builder:
                _storm_widgetbook_use_cases_storm_input.stormInputMultiline,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'password (obscured)',
            builder: _storm_widgetbook_use_cases_storm_input.stormInputObscured,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'with text',
            builder: _storm_widgetbook_use_cases_storm_input.stormInputText,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'StormToggle',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder:
                _storm_widgetbook_use_cases_storm_toggle.stormToggleDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'disabled',
            builder:
                _storm_widgetbook_use_cases_storm_toggle.stormToggleDisabled,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'off',
            builder: _storm_widgetbook_use_cases_storm_toggle.stormToggleOff,
          ),
        ],
      ),
    ],
  ),
  _widgetbook.WidgetbookCategory(
    name: 'Index',
    children: [
      _widgetbook.WidgetbookComponent(
        name: 'CandidateSite',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'one-off candidates',
            builder: _storm_widgetbook_index_usage_index.oneOffCandidateIndex,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'UsageSite',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'shared components',
            builder: _storm_widgetbook_index_usage_index.sharedUsageIndex,
          ),
        ],
      ),
    ],
  ),
  _widgetbook.WidgetbookCategory(
    name: 'Panels',
    children: [
      _widgetbook.WidgetbookComponent(
        name: 'InlineConfirm',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder:
                _storm_widgetbook_use_cases_inline_confirm.inlineConfirmDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long text (overflow)',
            builder:
                _storm_widgetbook_use_cases_inline_confirm.inlineConfirmLong,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'KeyValueList',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder:
                _storm_widgetbook_use_cases_key_value_list.keyValueListDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'empty',
            builder:
                _storm_widgetbook_use_cases_key_value_list.keyValueListEmpty,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long text (overflow)',
            builder:
                _storm_widgetbook_use_cases_key_value_list.keyValueListLong,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'MonoTag',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_panels.monoTagDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long text (overflow)',
            builder: _storm_widgetbook_use_cases_panels.monoTagLong,
          ),
        ],
      ),
    ],
  ),
  _widgetbook.WidgetbookCategory(
    name: 'Settings',
    children: [
      _widgetbook.WidgetbookComponent(
        name: 'BetaTag',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder:
                _storm_widgetbook_use_cases_settings_widgets.betaTagDefault,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'GroupLabel',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder:
                _storm_widgetbook_use_cases_settings_widgets.groupLabelDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long text (overflow)',
            builder:
                _storm_widgetbook_use_cases_settings_widgets.groupLabelLong,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'InfoBox',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder:
                _storm_widgetbook_use_cases_settings_widgets.infoBoxDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long text (overflow)',
            builder: _storm_widgetbook_use_cases_settings_widgets.infoBoxLong,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'MonoValue',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder:
                _storm_widgetbook_use_cases_settings_widgets.monoValueDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long text (overflow)',
            builder: _storm_widgetbook_use_cases_settings_widgets.monoValueLong,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'RowActions',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder:
                _storm_widgetbook_use_cases_settings_widgets.rowActionsDefault,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'SettingsButtonRow',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_settings_widgets
                .settingsButtonRowDefault,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'SettingsDivider',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_settings_widgets
                .settingsDividerDefault,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'SettingsMuted',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_settings_widgets
                .settingsMutedDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'error',
            builder:
                _storm_widgetbook_use_cases_settings_widgets.settingsMutedError,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long text (overflow)',
            builder:
                _storm_widgetbook_use_cases_settings_widgets.settingsMutedLong,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'SettingsNote',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_settings_widgets
                .settingsNoteDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long text (overflow)',
            builder:
                _storm_widgetbook_use_cases_settings_widgets.settingsNoteLong,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'TextAction',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder:
                _storm_widgetbook_use_cases_settings_widgets.textActionDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'disabled',
            builder:
                _storm_widgetbook_use_cases_settings_widgets.textActionDisabled,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'every variant',
            builder:
                _storm_widgetbook_use_cases_settings_widgets.textActionVariants,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long text (overflow)',
            builder:
                _storm_widgetbook_use_cases_settings_widgets.textActionLong,
          ),
        ],
      ),
    ],
  ),
  _widgetbook.WidgetbookCategory(
    name: 'Sidebar',
    children: [
      _widgetbook.WidgetbookComponent(
        name: 'SidebarRow',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_sidebar_rows.sidebarRowDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'disabled (no onTap)',
            builder:
                _storm_widgetbook_use_cases_sidebar_rows.sidebarRowDisabled,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long text (overflow)',
            builder: _storm_widgetbook_use_cases_sidebar_rows.sidebarRowLong,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'nested with guides',
            builder: _storm_widgetbook_use_cases_sidebar_rows.sidebarRowNested,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'selected',
            builder:
                _storm_widgetbook_use_cases_sidebar_rows.sidebarRowSelected,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'unseen change',
            builder: _storm_widgetbook_use_cases_sidebar_rows.sidebarRowUnseen,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'Twisty',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_sidebar_rows.twistyDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'open and closed',
            builder: _storm_widgetbook_use_cases_sidebar_rows.twistyBoth,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'UnseenDot',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_sidebar_rows.unseenDotDefault,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'VaultTag',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_sidebar_rows.vaultTagDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long text (overflow)',
            builder: _storm_widgetbook_use_cases_sidebar_rows.vaultTagLong,
          ),
        ],
      ),
    ],
  ),
  _widgetbook.WidgetbookCategory(
    name: 'States',
    children: [
      _widgetbook.WidgetbookComponent(
        name: 'ConflictCard',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_states.conflictCardDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'not dismissible',
            builder: _storm_widgetbook_use_cases_states.conflictCardFixed,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'EmptyState',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_states.emptyStateDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long text (overflow)',
            builder: _storm_widgetbook_use_cases_states.emptyStateLong,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'title only',
            builder: _storm_widgetbook_use_cases_states.emptyStateTitleOnly,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'OfflineNotice',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_states.offlineNoticeDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'edits queued, retry',
            builder: _storm_widgetbook_use_cases_states.offlineNoticeQueued,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'one edit queued',
            builder: _storm_widgetbook_use_cases_states.offlineNoticeOne,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'SkeletonRows',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default (loading)',
            builder: _storm_widgetbook_use_cases_states.skeletonRowsDefault,
          ),
        ],
      ),
    ],
  ),
  _widgetbook.WidgetbookCategory(
    name: 'Surfaces',
    children: [
      _widgetbook.WidgetbookComponent(
        name: 'PopoverDivider',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_popover.popoverDividerDefault,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'PopoverItem',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_popover.popoverItemDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'disabled (no onTap)',
            builder: _storm_widgetbook_use_cases_popover.popoverItemDisabled,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'every tone',
            builder: _storm_widgetbook_use_cases_popover.popoverItemTones,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long text (overflow)',
            builder: _storm_widgetbook_use_cases_popover.popoverItemLong,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'selected',
            builder: _storm_widgetbook_use_cases_popover.popoverItemSelected,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'SheetHost',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_storm_sheet.sheetHostDefault,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'StormPopover',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_popover.stormPopoverDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long text (overflow)',
            builder: _storm_widgetbook_use_cases_popover.stormPopoverLong,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'StormSheet',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'adaptive sheet (opens)',
            builder: _storm_widgetbook_use_cases_storm_sheet.stormSheetAdaptive,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_storm_sheet.stormSheetDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long text (overflow)',
            builder: _storm_widgetbook_use_cases_storm_sheet.stormSheetLong,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'no close affordance',
            builder: _storm_widgetbook_use_cases_storm_sheet.stormSheetNoClose,
          ),
        ],
      ),
    ],
  ),
  _widgetbook.WidgetbookCategory(
    name: 'Widgets',
    children: [
      _widgetbook.WidgetbookComponent(
        name: 'BrandMark',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_brand_mark.brandMarkDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'mark only',
            builder: _storm_widgetbook_use_cases_brand_mark.brandMarkOnly,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'Breadcrumb',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_breadcrumb.breadcrumbDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long trail (scrolls)',
            builder: _storm_widgetbook_use_cases_breadcrumb.breadcrumbLong,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'single crumb',
            builder: _storm_widgetbook_use_cases_breadcrumb.breadcrumbSingle,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'CodeBlock',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_code_block.codeBlockDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long text (scrolls)',
            builder: _storm_widgetbook_use_cases_code_block.codeBlockLong,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'CountBadge',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'capped (99+ / 9+)',
            builder: _storm_widgetbook_use_cases_count_badge.countBadgeCapped,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_count_badge.countBadgeDefault,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'FolderRow',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_folder_row.folderRowDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'disabled (no onTap)',
            builder: _storm_widgetbook_use_cases_folder_row.folderRowDisabled,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long text (overflow)',
            builder: _storm_widgetbook_use_cases_folder_row.folderRowLong,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'Hairline',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_hairline.hairlineDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'edges',
            builder: _storm_widgetbook_use_cases_hairline.hairlineEdges,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long text (overflow)',
            builder: _storm_widgetbook_use_cases_hairline.hairlineLong,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'IconBadgeButton',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder:
                _storm_widgetbook_use_cases_count_badge.iconBadgeButtonDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'many unread (capped)',
            builder:
                _storm_widgetbook_use_cases_count_badge.iconBadgeButtonCapped,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'no unread',
            builder:
                _storm_widgetbook_use_cases_count_badge.iconBadgeButtonQuiet,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'ImageThumbnail',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_image_thumbnail
                .imageThumbnailDefault,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'InboxHeader',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'all read',
            builder: _storm_widgetbook_use_cases_inbox_header.inboxHeaderRead,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder:
                _storm_widgetbook_use_cases_inbox_header.inboxHeaderDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long text (overflow)',
            builder: _storm_widgetbook_use_cases_inbox_header.inboxHeaderLong,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'KeyChip',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_tag_chip.keyChipDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long text (overflow)',
            builder: _storm_widgetbook_use_cases_tag_chip.keyChipLong,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'KindBadge',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'custom glyph',
            builder: _storm_widgetbook_use_cases_kind_badge.kindBadgeGlyph,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_kind_badge.kindBadgeDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'every kind',
            builder: _storm_widgetbook_use_cases_kind_badge.kindBadgeKinds,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'NoteRow',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_note_row.noteRowDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'disabled (no onTap)',
            builder: _storm_widgetbook_use_cases_note_row.noteRowDisabled,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long text (overflow)',
            builder: _storm_widgetbook_use_cases_note_row.noteRowLong,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'selected',
            builder: _storm_widgetbook_use_cases_note_row.noteRowSelected,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'NoticeBanner',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder:
                _storm_widgetbook_use_cases_notice_banner.noticeBannerDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'warning and strong',
            builder:
                _storm_widgetbook_use_cases_notice_banner.noticeBannerTones,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'NotificationGroupRow',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_notification_row
                .notificationGroupRowDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'expanded',
            builder: _storm_widgetbook_use_cases_notification_row
                .notificationGroupRowExpanded,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long text (overflow)',
            builder: _storm_widgetbook_use_cases_notification_row
                .notificationGroupRowLong,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'unread',
            builder: _storm_widgetbook_use_cases_notification_row
                .notificationGroupRowUnread,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'NotificationRow',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_notification_row
                .notificationRowDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'disabled (no onTap)',
            builder: _storm_widgetbook_use_cases_notification_row
                .notificationRowDisabled,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'focused (keyboard ring)',
            builder: _storm_widgetbook_use_cases_notification_row
                .notificationRowFocused,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'held tag and quoted body',
            builder: _storm_widgetbook_use_cases_notification_row
                .notificationRowRich,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long text (overflow)',
            builder: _storm_widgetbook_use_cases_notification_row
                .notificationRowLong,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'read',
            builder: _storm_widgetbook_use_cases_notification_row
                .notificationRowRead,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'OutlineTag',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_outline_tag.outlineTagDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'focused',
            builder: _storm_widgetbook_use_cases_outline_tag.outlineTagFocused,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'leading and trailing',
            builder: _storm_widgetbook_use_cases_outline_tag.outlineTagAdorned,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long label (overflow)',
            builder: _storm_widgetbook_use_cases_outline_tag.outlineTagLong,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'shapes and fill',
            builder: _storm_widgetbook_use_cases_outline_tag.outlineTagShapes,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'tappable',
            builder: _storm_widgetbook_use_cases_outline_tag.outlineTagTappable,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'tones',
            builder: _storm_widgetbook_use_cases_outline_tag.outlineTagTones,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'ReadingMeasure',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_reading_measure
                .readingMeasureDefault,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'SaveStateLabel',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_save_state_label
                .saveStateLabelDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'every tone',
            builder: _storm_widgetbook_use_cases_save_state_label
                .saveStateLabelTones,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long text (overflow)',
            builder:
                _storm_widgetbook_use_cases_save_state_label.saveStateLabelLong,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'SectionLabel',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder:
                _storm_widgetbook_use_cases_section_label.sectionLabelDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long text (overflow)',
            builder: _storm_widgetbook_use_cases_section_label.sectionLabelLong,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'SessionStatusChip',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_session_status
                .sessionStatusChipDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'every status',
            builder:
                _storm_widgetbook_use_cases_session_status.sessionStatusChipAll,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'unrecognised status',
            builder: _storm_widgetbook_use_cases_session_status
                .sessionStatusChipUnknown,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'SessionStatusDot',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_session_status
                .sessionStatusDotDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'every status',
            builder:
                _storm_widgetbook_use_cases_session_status.sessionStatusDotAll,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'StatusDot',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'custom colour and hollow',
            builder: _storm_widgetbook_use_cases_status_dot.statusDotCustom,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_status_dot.statusDotDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'every status',
            builder: _storm_widgetbook_use_cases_status_dot.statusDotAll,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'StatusPill',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder:
                _storm_widgetbook_use_cases_session_status.statusPillDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'error',
            builder: _storm_widgetbook_use_cases_session_status.statusPillError,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long label',
            builder: _storm_widgetbook_use_cases_session_status.statusPillLong,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'StormCard',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'colours and no border',
            builder: _storm_widgetbook_use_cases_storm_card.stormCardColours,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_storm_card.stormCardDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'densities',
            builder: _storm_widgetbook_use_cases_storm_card.stormCardDensities,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'focused',
            builder: _storm_widgetbook_use_cases_storm_card.stormCardFocused,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long text (overflow)',
            builder: _storm_widgetbook_use_cases_storm_card.stormCardLong,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'tappable',
            builder: _storm_widgetbook_use_cases_storm_card.stormCardTappable,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'StormIcon',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_storm_icon.stormIconDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'every glyph',
            builder: _storm_widgetbook_use_cases_storm_icon.stormIconAll,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'TagChip',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_tag_chip.tagChipDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long text (overflow)',
            builder: _storm_widgetbook_use_cases_tag_chip.tagChipLong,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'tappable',
            builder: _storm_widgetbook_use_cases_tag_chip.tagChipTappable,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'TagGroup',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_tag_group.tagGroupDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'empty',
            builder: _storm_widgetbook_use_cases_tag_group.tagGroupEmpty,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'many tags (wraps)',
            builder: _storm_widgetbook_use_cases_tag_group.tagGroupMany,
          ),
        ],
      ),
      _widgetbook.WidgetbookComponent(
        name: 'Tappable',
        useCases: [
          _widgetbook.WidgetbookUseCase(
            name: 'default',
            builder: _storm_widgetbook_use_cases_tappable.tappableDefault,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'disabled',
            builder: _storm_widgetbook_use_cases_tappable.tappableDisabled,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'focused',
            builder: _storm_widgetbook_use_cases_tappable.tappableFocused,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'long label (overflow)',
            builder: _storm_widgetbook_use_cases_tappable.tappableLong,
          ),
          _widgetbook.WidgetbookUseCase(
            name: 'small target',
            builder: _storm_widgetbook_use_cases_tappable.tappableSmall,
          ),
        ],
      ),
    ],
  ),
];
