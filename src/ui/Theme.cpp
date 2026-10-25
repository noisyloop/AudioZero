#include "Theme.h"
#include <juce_audio_utils/juce_audio_utils.h>

namespace az
{

Theme::Theme()
{
    using namespace theme;
    setColour(juce::ResizableWindow::backgroundColourId, bg0);
    setColour(juce::DocumentWindow::backgroundColourId, bg0);
    setColour(juce::TextButton::buttonColourId, bg2);
    setColour(juce::TextButton::buttonOnColourId, accent);
    setColour(juce::TextButton::textColourOffId, text);
    setColour(juce::TextButton::textColourOnId, juce::Colour(0xff15120f));
    setColour(juce::ComboBox::backgroundColourId, bg2);
    setColour(juce::ComboBox::textColourId, text);
    setColour(juce::ComboBox::outlineColourId, line);
    setColour(juce::ComboBox::arrowColourId, textDim);
    setColour(juce::PopupMenu::backgroundColourId, bg2);
    setColour(juce::PopupMenu::textColourId, text);
    setColour(juce::PopupMenu::highlightedBackgroundColourId, accent.withAlpha(0.85f));
    setColour(juce::PopupMenu::highlightedTextColourId, juce::Colours::black);
    setColour(juce::PopupMenu::headerTextColourId, textDim);
    setColour(juce::Label::textColourId, text);
    setColour(juce::Label::textWhenEditingColourId, text);
    setColour(juce::Label::backgroundWhenEditingColourId, bg0);
    setColour(juce::Label::outlineWhenEditingColourId, accent);
    setColour(juce::TextEditor::backgroundColourId, bg0);
    setColour(juce::TextEditor::textColourId, text);
    setColour(juce::TextEditor::outlineColourId, line);
    setColour(juce::TextEditor::focusedOutlineColourId, accent);
    setColour(juce::TextEditor::highlightColourId, accent.withAlpha(0.4f));
    setColour(juce::CaretComponent::caretColourId, accent);
    setColour(juce::Slider::thumbColourId, accent);
    setColour(juce::Slider::trackColourId, accent);
    setColour(juce::Slider::backgroundColourId, bg3);
    setColour(juce::Slider::rotarySliderFillColourId, accent);
    setColour(juce::Slider::rotarySliderOutlineColourId, bg3);
    setColour(juce::Slider::textBoxTextColourId, text);
    setColour(juce::Slider::textBoxOutlineColourId, juce::Colours::transparentBlack);
    setColour(juce::Slider::textBoxBackgroundColourId, bg0);
    setColour(juce::ListBox::backgroundColourId, bg1);
    setColour(juce::ListBox::textColourId, text);
    setColour(juce::ScrollBar::thumbColourId, bg3);
    setColour(juce::TooltipWindow::backgroundColourId, bg3);
    setColour(juce::TooltipWindow::textColourId, text);
    setColour(juce::TooltipWindow::outlineColourId, line);
    setColour(juce::AlertWindow::backgroundColourId, bg1);
    setColour(juce::AlertWindow::textColourId, text);
    setColour(juce::AlertWindow::outlineColourId, line);
    setColour(juce::BubbleComponent::backgroundColourId, bg3);
    setColour(juce::BubbleComponent::outlineColourId, line);
    setColour(juce::TabbedButtonBar::tabOutlineColourId, juce::Colours::transparentBlack);
    setColour(juce::TabbedComponent::outlineColourId, juce::Colours::transparentBlack);
    setColour(juce::TreeView::backgroundColourId, bg1);
    setColour(juce::MidiKeyboardComponent::keyDownOverlayColourId, accent.withAlpha(0.7f));
    setColour(juce::MidiKeyboardComponent::mouseOverKeyOverlayColourId, accent.withAlpha(0.25f));
    setColour(juce::DirectoryContentsDisplayComponent::highlightColourId, accent.withAlpha(0.5f));

    auto scheme = getDarkColourScheme();
    scheme.setUIColour(ColourScheme::windowBackground, bg1);
    scheme.setUIColour(ColourScheme::widgetBackground, bg2);
    scheme.setUIColour(ColourScheme::menuBackground, bg2);
    scheme.setUIColour(ColourScheme::outline, line);
    scheme.setUIColour(ColourScheme::defaultText, text);
    scheme.setUIColour(ColourScheme::defaultFill, accent);
    scheme.setUIColour(ColourScheme::highlightedText, juce::Colours::black);
    scheme.setUIColour(ColourScheme::highlightedFill, accent);
    scheme.setUIColour(ColourScheme::menuText, text);
    setColourScheme(scheme);
}

void Theme::drawRotarySlider(juce::Graphics& g, int x, int y, int w, int h, float pos, float start, float end, juce::Slider& slider)
{
    const auto bounds = juce::Rectangle<int>(x, y, w, h).toFloat().reduced(3.0f);
    const float radius = std::min(bounds.getWidth(), bounds.getHeight()) * 0.5f;
    const auto centre = bounds.getCentre();
    const float angle = start + pos * (end - start);
    const auto colour = slider.findColour(juce::Slider::rotarySliderFillColourId);
    const float stroke = std::max(2.5f, radius * 0.16f);

    juce::Path track;
    track.addCentredArc(centre.x, centre.y, radius - stroke * 0.5f, radius - stroke * 0.5f, 0.0f, start, end, true);
    g.setColour(theme::bg3);
    g.strokePath(track, juce::PathStrokeType(stroke, juce::PathStrokeType::curved, juce::PathStrokeType::rounded));

    // Bipolar parameters fill from the middle.
    const bool bipolar = slider.getMinimum() < 0.0 && slider.getMaximum() > 0.0;
    const float from = bipolar ? (start + end) * 0.5f : start;
    juce::Path value;
    value.addCentredArc(centre.x, centre.y, radius - stroke * 0.5f, radius - stroke * 0.5f, 0.0f, from, angle, true);
    g.setColour(slider.isEnabled() ? colour : theme::textDim);
    g.strokePath(value, juce::PathStrokeType(stroke, juce::PathStrokeType::curved, juce::PathStrokeType::rounded));

    const float inner = radius - stroke * 1.6f;
    g.setColour(theme::bg2.brighter(0.08f));
    g.fillEllipse(centre.x - inner, centre.y - inner, inner * 2.0f, inner * 2.0f);
    const juce::Point<float> tip(centre.x + std::sin(angle) * inner * 0.8f, centre.y - std::cos(angle) * inner * 0.8f);
    g.setColour(theme::text);
    g.drawLine(juce::Line<float>(centre.getPointOnCircumference(inner * 0.2f, angle), tip), 2.0f);
}

void Theme::drawLinearSlider(juce::Graphics& g, int x, int y, int w, int h, float pos, float, float,
                             juce::Slider::SliderStyle style, juce::Slider& slider)
{
    const bool vertical = style == juce::Slider::LinearVertical || style == juce::Slider::LinearBarVertical;
    const auto colour = slider.findColour(juce::Slider::trackColourId);
    auto area = juce::Rectangle<int>(x, y, w, h).toFloat();
    if (vertical)
    {
        const auto trackArea = area.withSizeKeepingCentre(4.0f, area.getHeight());
        g.setColour(theme::bg0);
        g.fillRoundedRectangle(trackArea, 2.0f);
        g.setColour(colour.withAlpha(0.8f));
        g.fillRoundedRectangle(trackArea.withTop(pos), 2.0f);
        const auto thumb = juce::Rectangle<float>(area.getWidth() * 0.8f, 10.0f).withCentre({ area.getCentreX(), pos });
        g.setColour(theme::bg3.brighter(0.3f));
        g.fillRoundedRectangle(thumb, 3.0f);
        g.setColour(theme::text);
        g.drawLine(thumb.getX() + 3, thumb.getCentreY(), thumb.getRight() - 3, thumb.getCentreY(), 1.5f);
    }
    else
    {
        const auto trackArea = area.withSizeKeepingCentre(area.getWidth(), 4.0f);
        g.setColour(theme::bg0);
        g.fillRoundedRectangle(trackArea, 2.0f);
        g.setColour(colour.withAlpha(0.8f));
        g.fillRoundedRectangle(trackArea.withRight(pos), 2.0f);
        g.setColour(theme::text);
        g.fillEllipse(juce::Rectangle<float>(9.0f, 9.0f).withCentre({ pos, area.getCentreY() }));
    }
}

void Theme::drawButtonBackground(juce::Graphics& g, juce::Button& button, const juce::Colour& background, bool highlighted, bool down)
{
    auto bounds = button.getLocalBounds().toFloat().reduced(0.5f);
    auto colour = button.getToggleState() ? button.findColour(juce::TextButton::buttonOnColourId) : background;
    if (down)
        colour = colour.brighter(0.15f);
    else if (highlighted)
        colour = colour.brighter(0.07f);
    g.setColour(colour);
    g.fillRoundedRectangle(bounds, 4.0f);
    if (! button.getToggleState())
    {
        g.setColour(theme::line);
        g.drawRoundedRectangle(bounds, 4.0f, 1.0f);
    }
}

void Theme::drawButtonText(juce::Graphics& g, juce::TextButton& button, bool, bool)
{
    g.setFont(theme::font(std::min(13.0f, button.getHeight() * 0.62f), button.getToggleState()));
    g.setColour(button.findColour(button.getToggleState() ? juce::TextButton::textColourOnId : juce::TextButton::textColourOffId)
                    .withMultipliedAlpha(button.isEnabled() ? 1.0f : 0.4f));
    g.drawFittedText(button.getButtonText(), button.getLocalBounds().reduced(3, 0), juce::Justification::centred, 1);
}

void Theme::drawToggleButton(juce::Graphics& g, juce::ToggleButton& button, bool highlighted, bool)
{
    auto bounds = button.getLocalBounds().toFloat();
    auto box = bounds.removeFromLeft(bounds.getHeight()).reduced(4.0f);
    g.setColour(button.getToggleState() ? theme::accent : (highlighted ? theme::bg3 : theme::bg2));
    g.fillRoundedRectangle(box, 3.0f);
    g.setColour(theme::line);
    g.drawRoundedRectangle(box, 3.0f, 1.0f);
    g.setColour(theme::text);
    g.setFont(theme::font(13.0f));
    g.drawText(button.getButtonText(), bounds.reduced(2.0f, 0.0f), juce::Justification::centredLeft);
}

void Theme::drawComboBox(juce::Graphics& g, int w, int h, bool, int, int, int, int, juce::ComboBox& box)
{
    auto bounds = juce::Rectangle<float>(0, 0, static_cast<float>(w), static_cast<float>(h)).reduced(0.5f);
    g.setColour(box.findColour(juce::ComboBox::backgroundColourId));
    g.fillRoundedRectangle(bounds, 4.0f);
    g.setColour(box.hasKeyboardFocus(false) ? theme::accent : theme::line);
    g.drawRoundedRectangle(bounds, 4.0f, 1.0f);
    juce::Path arrow;
    const float cx = static_cast<float>(w) - 11.0f, cy = static_cast<float>(h) * 0.5f;
    arrow.addTriangle(cx - 4.0f, cy - 2.0f, cx + 4.0f, cy - 2.0f, cx, cy + 3.0f);
    g.setColour(theme::textDim);
    g.fillPath(arrow);
}

void Theme::drawScrollbar(juce::Graphics& g, juce::ScrollBar&, int x, int y, int w, int h, bool vertical, int thumbStart,
                          int thumbSize, bool over, bool down)
{
    juce::Rectangle<int> thumb = vertical ? juce::Rectangle<int>(x, thumbStart, w, thumbSize) : juce::Rectangle<int>(thumbStart, y, thumbSize, h);
    g.setColour(theme::bg3.brighter(down ? 0.3f : (over ? 0.15f : 0.0f)));
    g.fillRoundedRectangle(thumb.toFloat().reduced(2.0f), 3.0f);
}

void Theme::drawTabButton(juce::TabBarButton& button, juce::Graphics& g, bool over, bool)
{
    auto area = button.getActiveArea().toFloat();
    const bool front = button.isFrontTab();
    g.setColour(front ? theme::bg2 : (over ? theme::bg1.brighter(0.05f) : theme::bg1));
    g.fillRect(area);
    if (front)
    {
        g.setColour(theme::accent);
        g.fillRect(area.removeFromBottom(2.0f));
    }
    g.setColour(front ? theme::text : theme::textDim);
    g.setFont(theme::font(13.0f, front));
    g.drawText(button.getButtonText(), button.getActiveArea(), juce::Justification::centred);
}

//==============================================================================
Knob::Knob(juce::RangedAudioParameter& parameter, const juce::String& text, juce::Colour colour)
    : label(text.isNotEmpty() ? text : parameter.getName(16))
{
    slider.setSliderStyle(juce::Slider::RotaryHorizontalVerticalDrag);
    slider.setTextBoxStyle(juce::Slider::NoTextBox, false, 0, 0);
    slider.setColour(juce::Slider::rotarySliderFillColourId, colour);
    slider.setPopupDisplayEnabled(true, true, nullptr);
    slider.setTooltip(parameter.getName(64));
    addAndMakeVisible(slider);
    attachment = std::make_unique<juce::SliderParameterAttachment>(parameter, slider);
    slider.setDoubleClickReturnValue(true, parameter.convertFrom0to1(parameter.getDefaultValue()));
}

void Knob::resized()
{
    auto area = getLocalBounds();
    area.removeFromBottom(14);
    slider.setBounds(area);
}

void Knob::paint(juce::Graphics& g)
{
    g.setColour(theme::textDim);
    g.setFont(theme::font(10.5f));
    g.drawFittedText(label, getLocalBounds().removeFromBottom(14), juce::Justification::centred, 1, 0.8f);
}

ParamToggle::ParamToggle(juce::RangedAudioParameter& parameter, const juce::String& label)
{
    button.setButtonText(label);
    button.setClickingTogglesState(true);
    addAndMakeVisible(button);
    attachment = std::make_unique<juce::ButtonParameterAttachment>(parameter, button);
}

ParamChoice::ParamChoice(juce::RangedAudioParameter& parameter)
{
    if (auto* choice = dynamic_cast<juce::AudioParameterChoice*>(&parameter))
        box.addItemList(choice->choices, 1);
    addAndMakeVisible(box);
    attachment = std::make_unique<juce::ComboBoxParameterAttachment>(parameter, box);
}

} // namespace az
