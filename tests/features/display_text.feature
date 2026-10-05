Feature: Displaying text on a paired Screensight device
  As a Home Assistant user
  I want to change the text shown on the panel
  So that my display reacts to my automations

  Scenario: Setting the Display text entity pushes a frame to the device
    Given a paired Screensight device is connected
    When I set the Display text entity to "Hello 🌧"
    Then the device receives a set_text frame with "Hello 🌧"
