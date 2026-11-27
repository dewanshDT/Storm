// dart format width=80
// coverage:ignore-file
// ignore_for_file: type=lint
// ignore_for_file: unused_import, prefer_relative_imports, directives_ordering

// GENERATED CODE - DO NOT MODIFY BY HAND

// **************************************************************************
// AppGenerator
// **************************************************************************

// ignore_for_file: no_leading_underscores_for_library_prefixes

import 'package:storm_widgetbook/use_cases/choice_cards.dart'
    as _storm_widgetbook_use_cases_choice_cards;
import 'package:storm_widgetbook/use_cases/choice_chips.dart'
    as _storm_widgetbook_use_cases_choice_chips;
import 'package:storm_widgetbook/use_cases/inline_confirm.dart'
    as _storm_widgetbook_use_cases_inline_confirm;
import 'package:storm_widgetbook/use_cases/key_value_list.dart'
    as _storm_widgetbook_use_cases_key_value_list;
import 'package:storm_widgetbook/use_cases/storm_button.dart'
    as _storm_widgetbook_use_cases_storm_button;
import 'package:storm_widgetbook/use_cases/storm_checkbox.dart'
    as _storm_widgetbook_use_cases_storm_checkbox;
import 'package:widgetbook/widgetbook.dart' as _widgetbook;

final directories = <_widgetbook.WidgetbookNode>[
  _widgetbook.WidgetbookCategory(
    name: 'Controls',
    children: [
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
        name: 'StormButton',
        useCases: [
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
            name: 'row (long text)',
            builder:
                _storm_widgetbook_use_cases_storm_checkbox.stormCheckboxRowLong,
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
    ],
  ),
];
